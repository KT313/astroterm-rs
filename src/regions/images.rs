//! The image caches of plan section 4: one sky-fixed tile per group (the group's brightness prefix drawn as the
//! screen would draw it) and one per region (its group images composited). Per-star zoom brightness is applied before blending; tone mapping
//! happens in the frame output.
use crate::formulas::Vector3;
use crate::constants::TILE_EDGE_TEXELS;
use crate::model::{StarOpacities, Texel, StarTable, GroupTileKey, GroupTileCache, RegionTileCache, StarGroup};
use crate::regions::{TileGeometry, UNPLACED, blend_star, draw_order_key, premultiplied_color};
use super::content_bounds::calculate_content_radius;
use super::filter_summary::build_filter_summary;

impl GroupTileKey {
    /// The key of an image whose prefix is empty: nothing to draw, whatever the threshold.
    pub const EMPTY: GroupTileKey = GroupTileKey { simulated_at_ms: 0, star_count: 0, magnitude_threshold: 0, zoom_boost: 1.0 };
}

/// A star placed on a group tile, waiting for its turn. Sorting these compact records instead of row indices
/// keeps the drawing loop from reading the star table out of order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PlacedGlyph {
    order: u64,                             // `draw_order_key`: compared first, and unique within the table
    bottom_right: u16,                      // texel index of the glyph's lower right texel
    magnitude_code: u16,
    color: u8,
}

impl GroupTileCache {
    pub fn key(&self) -> Option<GroupTileKey> { self.key }
    pub fn texels(&self) -> &[Texel] { &self.tile.texels }
    pub fn is_empty(&self) -> bool { self.tile.texels.is_empty() }
    pub fn bytes(&self) -> usize { self.tile.texels.len() * Texel::SIZE }

    /// Record an empty brightness prefix and release its old image without preparing drawing geometry.
    pub fn clear(&mut self) {
        self.key = Some(GroupTileKey::EMPTY);
        self.tile.texels = Vec::new();
    }

    /// Draw rows `0..key.star_count` of the simulation (`directions` and `magnitudes` at `key.simulated_at_ms`) into the tile: each
    /// row brighter than the threshold becomes the four-texel glyph of its colour, blended source-over from dim
    /// to bright current magnitude, then ascending ID. Catalog brightness-bound order is unchanged. Rows the model
    /// could not place and rows outside the tile are skipped.
    pub fn render(&mut self, key: GroupTileKey, tile: &TileGeometry, table: &StarTable, directions: &[Vector3], magnitudes: &[u16], opacities: &StarOpacities) {
        self.key = Some(key);
        if key.star_count == 0 { self.tile.texels = Vec::new(); return; }
        let size = TileGeometry::size();
        const { assert!(TILE_EDGE_TEXELS * TILE_EDGE_TEXELS <= 1 << 16) };                  // a texel index fits a glyph's u16
        self.tile.texels.clear();
        self.tile.texels.resize(TileGeometry::texel_count(), Texel::default());

        // Place every drawable row of the prefix in table order, then sort only the placed glyphs into drawing order.
        let columns = table.columns();
        let mut glyphs = Vec::with_capacity(key.star_count);
        for (row, &code) in magnitudes[..key.star_count].iter().enumerate() {
            if code > key.magnitude_threshold || code == UNPLACED { continue; }
            let Some((column, line)) = tile.project(directions[row]) else { continue };
            let (x, y) = (round_half_away(column), round_half_away(line));
            if x < 1 || y < 1 || x >= size as i64 || y >= size as i64 { continue; }            // the four-texel footprint must fit, as on the screen
            let bottom_right = (y as usize * size + x as usize) as u16;
            glyphs.push(PlacedGlyph { order: draw_order_key(code, columns.id[row]), bottom_right, magnitude_code: code, color: columns.display_color[row] });
        }
        glyphs.sort_unstable();

        // Blend the glyphs source-over, dimmest first.
        for glyph in glyphs {
            let opacity = opacities.of(glyph.magnitude_code);
            let premultiplied = premultiplied_color(glyph.color, opacity);
            let bottom_right = usize::from(glyph.bottom_right);
            for offset in [bottom_right - size - 1, bottom_right - size, bottom_right - 1, bottom_right] {
                blend_star(&mut self.tile.texels[offset], premultiplied, opacity);
            }
        }
        #[cfg(feature = "pixel-debug")]
        for texel in &self.tile.texels { texel.check("GroupTileCache::render output", 1.0); }
    }
}

/// `v.round() as i64` without the library call, as the screen projection rounds.
#[inline]
fn round_half_away(v: f64) -> i64 { (v + 0.5_f64.copysign(v)) as i64 }

impl RegionTileCache {
    pub fn texels(&self) -> &[Texel] { &self.tile.texels }
    pub fn is_empty(&self) -> bool { self.tile.texels.is_empty() }
    /// Populated image storage, including masks, bounds and filtering data. Empty images hold no payload.
    pub fn bytes(&self) -> usize {
        let image_bytes = self.tile.texels.len() * Texel::SIZE;
        let mask_bytes = self.lit_rows.len() * std::mem::size_of::<u64>();
        let bound_bytes = if self.tile.texels.is_empty() { 0 } else { std::mem::size_of_val(&self.content_radius) };
        let summary_bytes = if self.tile.texels.is_empty() { 0 } else {
            let blocks = self.filter_summary.as_ref().map_or(0, |summary| summary.block_sums.len());
            std::mem::size_of_val(&self.filter_summary) + blocks * std::mem::size_of::<[f64; 4]>()
        };
        image_bytes + mask_bytes + bound_bytes + summary_bytes
    }
    pub fn is_merged(&self) -> bool { self.group_keys.is_some() }
    /// One bit per texel, a word per tile row: which texels hold light. Empty when the image is.
    pub fn lit_rows(&self) -> &[u64] { &self.lit_rows }

    /// Whether the merge still matches the group images.
    pub fn is_current(&self, groups: &[StarGroup]) -> bool {
        self.group_keys.as_ref().is_some_and(|keys| keys.iter().zip(groups).all(|(key, group)| *key == group.tile.key()))
    }

    /// Merge `groups` in ascending speed order, weighting their already-premultiplied RGB by opacity again.
    pub fn merge(&mut self, groups: &[StarGroup]) {
        // Replace the image and all its derived data under the same group keys.
        self.group_keys = Some(std::array::from_fn(|group| groups[group].tile.key()));
        let mut parts = groups.iter().map(|group| &group.tile).filter(|image| !image.is_empty());
        let Some(first) = parts.next() else {
            self.tile.texels = Vec::new();
            self.lit_rows = Vec::new();
            self.content_radius = Some(0.0);
            self.filter_summary = None;
            return;
        };

        // Keep the existing slowest-to-fastest colour and opacity composition.
        #[cfg(feature = "pixel-debug")]
        for texel in first.texels() { texel.check("RegionTileCache::merge base", 1.0); }
        self.tile.texels.clear();
        self.tile.texels.extend_from_slice(first.texels());
        for part in parts { merge_group_texels(&mut self.tile.texels, part.texels()); }

        // Record which texels contain light and how far their filter support reaches from the centre.
        scan_lit_rows(&self.tile.texels, &mut self.lit_rows);
        self.content_radius = Some(calculate_content_radius(&self.lit_rows));
        self.filter_summary = Some(build_filter_summary(&self.tile.texels, &self.lit_rows));
    }
}

/// Merge one group image onto the merged texels. Normal x86-64 builds skip transparent inputs four at a time
/// with SSE2; pixel-debug builds keep the scalar path, which checks every texel, including transparent ones.
fn merge_group_texels(merged: &mut [Texel], next: &[Texel]) {
    #[cfg(all(target_arch = "x86_64", not(feature = "pixel-debug")))]
    super::merge_sse2::merge_group_texels(merged, next);
    #[cfg(any(not(target_arch = "x86_64"), feature = "pixel-debug"))]
    merge_group_texels_scalar(merged, next);
}

/// Record one bit per lit texel, one word per tile row; four texels per SSE2 test where available.
fn scan_lit_rows(texels: &[Texel], lit_rows: &mut Vec<u64>) {
    #[cfg(all(target_arch = "x86_64", not(feature = "pixel-debug")))]
    super::merge_sse2::scan_lit_rows(texels, lit_rows);
    #[cfg(any(not(target_arch = "x86_64"), feature = "pixel-debug"))]
    scan_lit_rows_scalar(texels, lit_rows);
}

/// Merge one group image onto the merged texels, texel by texel. Portable path and test oracle.
#[cfg(any(not(target_arch = "x86_64"), feature = "pixel-debug", test))]
fn merge_group_texels_scalar(merged: &mut [Texel], next: &[Texel]) {
    assert_eq!(merged.len(), next.len(), "a group image must match the merged image");
    for (base, texel) in merged.iter_mut().zip(next) { merge_group_texel(base, *texel); }
}

/// One bit per lit texel (opacity not equal to zero, so NaN counts as lit), one word per tile row. Portable path
/// and test oracle.
#[cfg(any(not(target_arch = "x86_64"), feature = "pixel-debug", test))]
fn scan_lit_rows_scalar(texels: &[Texel], lit_rows: &mut Vec<u64>) {
    lit_rows.clear();
    for row in texels.chunks_exact(crate::constants::TILE_EDGE_TEXELS) {
        let mut lit_columns = 0_u64;
        for (column, texel) in row.iter().enumerate() {
            if !texel.is_empty() { lit_columns |= 1 << column; }
        }
        lit_rows.push(lit_columns);
    }
}

/// Experimental owner-requested weighting of stored RGB, without unpremultiplying it first. Opacity still
/// accumulates with source-over. A transparent input contributes nothing; two empty texels stay empty.
pub(super) fn merge_group_texel(base: &mut Texel, next: Texel) {
    #[cfg(feature = "pixel-debug")]
    { base.check("merge_group_texel base", 1.0); next.check("merge_group_texel next", 1.0); }
    if next.opacity == 0.0 { return; }
    if base.opacity == 0.0 { *base = next; return; }
    let total = base.opacity + next.opacity;
    let (base_weight, next_weight) = (base.opacity / total, next.opacity / total);
    for (old, new) in base.rgb.iter_mut().zip(next.rgb) { *old = *old * base_weight + new * next_weight; }
    base.opacity = base.opacity * (1.0 - next.opacity) + next.opacity;
    #[cfg(feature = "pixel-debug")]
    base.check("merge_group_texel output", 1.0);
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod merge_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formulas::J2000;
    use crate::constants::{GRID_DEPTH, STAR_OPACITY_MAGNITUDE_SCALE, STAR_OPACITY_REFERENCE_MAGNITUDE};
    use crate::catalog::magnitude_code;
    use crate::model::{StarColor, StarRow, hash_direction};
    use crate::model::fixtures;
    use crate::regions::appearance::star_opacity;
    use crate::regions::UNPLACED;

    const SIZE: usize = 64;
    const MIDDLE: usize = 32;                                                               // the region centre at 31.5 rounds up: its glyph covers texels 31 and 32

    fn region() -> usize { hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.0, z: 0.0 }) }
    fn table(rows: &[StarRow]) -> StarTable {
        let stars = fixtures::layout(&rows.iter().map(|row| (region(), 0, *row)).collect::<Vec<_>>());
        stars.star_table(region(), 0).clone()
    }
    fn key(rows: usize, threshold: f64) -> GroupTileKey { GroupTileKey { simulated_at_ms: crate::formulas::utc_jd_to_time_ms(J2000).unwrap(), star_count: rows, magnitude_threshold: magnitude_code(threshold), zoom_boost: 1.0 } }
    fn opacities() -> StarOpacities { let mut table = StarOpacities::default(); table.prepare(1.0); table }
    fn lit(texels: &[Texel]) -> Vec<(usize, Texel)> { texels.iter().copied().enumerate().filter(|(_, texel)| !texel.is_empty()).collect() }

    #[test]
    fn a_star_at_the_region_centre_lights_the_four_texels_around_the_middle_with_its_colour_and_opacity() {
        let tile = TileGeometry::new(region());
        let mut row = fixtures::row(tile.center(), 0.0, 3.0, 7);
        row.display_color = StarColor::RedOrange.index();
        let table = table(&[row]);
        let mut image = GroupTileCache::default();
        image.render(key(1, 6.0), &tile, &table, &[tile.center()], &[magnitude_code(3.0)], &opacities());
        let lit = lit(image.texels());
        assert_eq!(lit.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [(MIDDLE - 1) * SIZE + MIDDLE - 1, (MIDDLE - 1) * SIZE + MIDDLE, MIDDLE * SIZE + MIDDLE - 1, MIDDLE * SIZE + MIDDLE]);
        let opacity = star_opacity(3.0, 1.0);
        for (_, texel) in lit {
            assert_eq!(texel.opacity, opacity);
            assert_eq!(texel.rgb, StarColor::RedOrange.rgb().map(|level| level as f32 / 255.0 * opacity));
        }
        assert_eq!(image.key(), Some(key(1, 6.0)));
        assert_eq!(image.bytes(), SIZE * SIZE * 16);
    }

    /// The group image as drawn before glyphs were placed first: rows sorted into drawing order, then projected.
    fn render_reference(key: GroupTileKey, tile: &TileGeometry, table: &StarTable, directions: &[Vector3], magnitudes: &[u16], opacities: &StarOpacities) -> Vec<Texel> {
        let size = TileGeometry::size();
        let mut texels = vec![Texel::default(); TileGeometry::texel_count()];
        let columns = table.columns();
        for row in crate::regions::order_star_rows(&magnitudes[..key.star_count], &columns.id[..key.star_count], key.magnitude_threshold) {
            let Some((column, line)) = tile.project(directions[row]) else { continue };
            let (x, y) = (round_half_away(column), round_half_away(line));
            if x < 1 || y < 1 || x >= size as i64 || y >= size as i64 { continue; }
            let opacity = opacities.of(magnitudes[row]);
            let premultiplied = premultiplied_color(columns.display_color[row], opacity);
            let bottom_right = y as usize * size + x as usize;
            for offset in [bottom_right - size - 1, bottom_right - size, bottom_right - 1, bottom_right] { blend_star(&mut texels[offset], premultiplied, opacity); }
        }
        texels
    }

    #[test]
    fn placed_glyphs_draw_the_bits_of_sorted_rows() {
        // Hundreds of overlapping stars with tied magnitudes, shuffled IDs and colours, some unplaced, some outside.
        let tile = TileGeometry::new(region());
        let middle = TileGeometry::middle();
        let mut seed = 7_u32;
        let mut next = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); seed };
        let count = 700;
        let directions: Vec<Vector3> = (0..count).map(|_| tile.direction_of(middle + (next() % 90) as f64 - 45.0 + 0.3, middle + (next() % 90) as f64 - 45.0 - 0.2)).collect();
        let rows: Vec<StarRow> = (0..count).map(|index| {
            let mut row = fixtures::row(directions[index], 0.0, 1.0 + (index % 5) as f64, (index as u32).wrapping_mul(2654435761) % 1_000_000);
            row.display_color = (next() % 7) as u8;
            row
        }).collect();
        let table = table(&rows);
        let magnitudes: Vec<u16> = (0..count).map(|_| match next() % 9 { 0 => UNPLACED, 1 => magnitude_code(9.0), code => magnitude_code(code as f64 * 0.5) }).collect();
        for (star_count, threshold) in [(count, 6.0), (count / 3, 3.0), (1, 6.0)] {
            let mut image = GroupTileCache::default();
            image.render(key(star_count, threshold), &tile, &table, &directions, &magnitudes, &opacities());
            let expected = render_reference(key(star_count, threshold), &tile, &table, &directions, &magnitudes, &opacities());
            let bits = |texels: &[Texel]| texels.iter().map(|texel| [texel.rgb[0].to_bits(), texel.rgb[1].to_bits(), texel.rgb[2].to_bits(), texel.opacity.to_bits()]).collect::<Vec<_>>();
            assert!(bits(image.texels()) == bits(&expected), "{star_count} rows, threshold {threshold}");
            if star_count == count { assert!(lit(image.texels()).len() > 500); }                 // many glyphs, overlapping
        }
    }

    #[test]
    fn rows_beyond_the_prefix_the_threshold_the_tile_or_the_model_are_not_drawn() {
        let tile = TileGeometry::new(region());
        let middle = TileGeometry::middle();
        let inside = tile.direction_of(middle - 8.0, middle + 6.0);
        let outside = tile.direction_of(-5.0, middle);
        let rows = [fixtures::row(tile.center(), 0.0, 3.0, 1), fixtures::row(inside, 0.0, 3.0, 2), fixtures::row(outside, 0.0, 3.0, 3), fixtures::row(inside, 0.0, 7.0, 4), fixtures::row(inside, 0.0, 3.0, 5)];
        let table = table(&rows);
        let directions = [tile.center(), inside, outside, inside, inside];
        let codes = [magnitude_code(3.0), magnitude_code(3.0), magnitude_code(3.0), magnitude_code(7.0), UNPLACED];
        let mut image = GroupTileCache::default();
        image.render(key(5, 6.0), &tile, &table, &directions, &codes, &opacities());
        assert_eq!(lit(image.texels()).len(), 8);                                            // the centre star and the inside star; the rest skipped
        image.render(key(1, 6.0), &tile, &table, &directions, &codes, &opacities());
        assert_eq!(lit(image.texels()).len(), 4);                                            // a shorter prefix
        image.render(key(5, 2.0), &tile, &table, &directions, &codes, &opacities());
        assert!(image.texels().iter().all(|texel| texel.is_empty()));                       // a brighter threshold: everything too faint, the tile still allocated
        image.render(GroupTileKey::EMPTY, &tile, &table, &directions, &codes, &opacities());
        assert!(image.is_empty() && image.bytes() == 0);                                     // an empty prefix holds no texels
    }

    #[test]
    fn stars_in_one_image_blend_source_over_as_on_the_screen() {
        let tile = TileGeometry::new(region());
        let faint = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;  // opacity 0.1
        let table = table(&[fixtures::row(tile.center(), 0.0, faint, 1), fixtures::row(tile.center(), 0.0, faint, 2)]);
        let mut image = GroupTileCache::default();
        image.render(key(2, faint + 1.0), &tile, &table, &[tile.center(); 2], &[magnitude_code(faint); 2], &opacities());
        let texel = image.texels()[MIDDLE * SIZE + MIDDLE];
        assert!((texel.opacity - 0.19).abs() < 1e-3, "{}", texel.opacity);                   // 0.1 over 0.1, not 0.2 (the code rounds the magnitude to a thousandth)
    }

    #[test]
    fn overlapping_stars_follow_current_brightness_and_id_without_reordering_the_catalog() {
        let tile = TileGeometry::new(region());
        let colors = [StarColor::RedOrange, StarColor::BlueWhite, StarColor::Orange];
        let rows: Vec<_> = [30, 10, 20].into_iter().enumerate().map(|(i, id)| {
            let mut row = fixtures::row(tile.center(), 0.0, i as f64, id);                    // conservative bounds ascend, independently of today's magnitudes
            row.display_color = colors[i].index();
            row
        }).collect();
        let table = table(&rows);
        let before = table.clone();
        let opacities = opacities();
        for (magnitudes, expected_order) in [([3.0, 0.0, 6.0], [2, 0, 1]), ([3.0; 3], [1, 2, 0])] {
            let codes = magnitudes.map(magnitude_code);
            let mut image = GroupTileCache::default();
            image.render(key(3, 9.0), &tile, &table, &[tile.center(); 3], &codes, &opacities);
            let mut expected = Texel::default();
            for row in expected_order {
                let opacity = opacities.of(codes[row]);
                blend_star(&mut expected, premultiplied_color(colors[row].index(), opacity), opacity);
            }
            let lit = lit(image.texels());
            assert_eq!(lit.len(), 4);
            assert!(lit.iter().all(|(_, texel)| *texel == expected), "current magnitudes {magnitudes:?}");
            assert_eq!(table, before);                                                       // sorting changes only temporary row indices
        }
    }

    #[test]
    fn group_rgb_is_weighted_by_opacity_in_slow_to_fast_order() {
        let group = |rgb: [f32; 3], opacity: f32| StarGroup {
            tile: GroupTileCache { key: Some(key(1, 9.0)), tile: crate::model::Tile { texels: vec![Texel { rgb: rgb.map(|c| c * opacity), opacity }; SIZE * SIZE] } },
            ..StarGroup::default()
        };
        let groups = [group([1.0, 0.0, 0.0], 0.2), group([0.0, 1.0, 0.0], 0.5), group([0.0, 0.0, 1.0], 0.25)];
        let mut image = RegionTileCache::default();
        image.merge(&groups);
        let actual = image.texels()[0];                                                  // red base, green over red, then blue over both
        let expected = Texel { rgb: [0.040336136, 0.25210086, 0.073529415], opacity: 0.7 };
        for (a, b) in actual.rgb.into_iter().chain([actual.opacity]).zip(expected.rgb.into_iter().chain([expected.opacity])) {
            assert!((a - b).abs() < 1e-6, "{actual:?} != {expected:?}");
        }
        let mut reversed = RegionTileCache::default();
        reversed.merge(&[groups[2].clone(), groups[1].clone(), groups[0].clone()]);
        assert_ne!(reversed.texels()[0].rgb, actual.rgb);                                 // speed order affects colour

        let transparent = group([0.0; 3], 0.0);
        image.merge(&[groups[0].clone(), StarGroup::default(), transparent]);
        assert_eq!(image.texels(), groups[0].tile.texels());                                  // empty or transparent groups leave the base intact
        let opaque = group([0.0, 0.0, 1.0], 1.0);
        image.merge(&[groups[0].clone(), groups[1].clone(), opaque.clone()]);
        assert_eq!(image.texels()[0].opacity, 1.0);
        assert!(image.texels()[0].rgb[0] > 0.0 && image.texels()[0].rgb[1] > 0.0);         // opaque input still mixes colours under the requested rule
        assert!((image.texels()[0].rgb[2] - 0.625).abs() < 1e-6);
    }

    #[test]
    fn stored_orange_and_blue_follow_the_requested_one_to_two_weights() {
        let mut base = Texel { rgb: [12.75 / 255.0, 11.55 / 255.0, 8.8 / 255.0], opacity: 0.05 };
        let next = Texel { rgb: [17.6 / 255.0, 22.9 / 255.0, 25.5 / 255.0], opacity: 0.1 };
        merge_group_texel(&mut base, next);
        for (actual, expected) in base.rgb.into_iter().zip([15.983333, 19.116667, 19.933334]) {
            assert!((actual * 255.0 - expected).abs() < 1e-5);
        }
        assert!((base.opacity - 0.145).abs() < 1e-6);
        let mut empty = Texel::default();
        merge_group_texel(&mut empty, Texel::default());
        assert_eq!(empty, Texel::default());
    }

    #[test]
    fn the_region_image_blends_its_group_images_and_tracks_their_keys() {
        let tile = TileGeometry::new(region());
        let faint = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;
        let table = table(&[fixtures::row(tile.center(), 0.0, faint, 1)]);
        let mut groups = vec![StarGroup::default(), StarGroup::default(), StarGroup::default()];
        groups[0].tile.render(key(1, faint + 1.0), &tile, &table, &[tile.center()], &[magnitude_code(faint)], &opacities());
        groups[2].tile.render(key(1, faint + 1.0), &tile, &table, &[tile.center()], &[magnitude_code(faint)], &opacities());
        let mut region_image = RegionTileCache::default();
        assert!(!region_image.is_current(&groups) && !region_image.is_merged());
        region_image.merge(&groups);
        assert!(region_image.is_current(&groups) && region_image.is_merged());
        let middle = MIDDLE * SIZE + MIDDLE;
        assert!((region_image.texels()[middle].opacity - 0.19).abs() < 1e-3);                 // source-over: 0.1 + 0.1 × (1 - 0.1)
        let base = groups[0].tile.texels()[middle];
        assert_eq!(region_image.texels()[middle].rgb, base.rgb);                          // identical stored RGB averages to itself
        let lit: Vec<(usize, u32)> = region_image.lit_rows().iter().enumerate().filter(|(_, bits)| **bits != 0).map(|(y, bits)| (y, bits.count_ones())).collect();
        assert_eq!(lit, [(MIDDLE - 1, 2), (MIDDLE, 2)]);                                     // the glyph's two rows, two texels each
        assert_eq!(region_image.lit_rows()[MIDDLE] & (1 << MIDDLE), 1 << MIDDLE);
        groups[2].tile.render(key(1, 2.0), &tile, &table, &[tile.center()], &[magnitude_code(faint)], &opacities());
        assert!(!region_image.is_current(&groups));                                          // a group key changed
        region_image.merge(&groups);
        assert!((region_image.texels()[middle].opacity - 0.1).abs() < 1e-3);
        region_image.merge(&[StarGroup::default(), StarGroup::default(), StarGroup::default()]);
        assert!(region_image.is_empty() && region_image.bytes() == 0 && region_image.is_merged() && region_image.lit_rows().is_empty()); // only empty groups: no texels
    }

    #[test]
    fn content_bounds_follow_merged_contents_and_release_on_empty_replacement() {
        // Give one group a central texel and merge it through the real image preparation path.
        let mut groups: [StarGroup; crate::constants::SPEED_GROUP_COUNT] = std::array::from_fn(|_| StarGroup::default());
        groups[0].tile.key = Some(key(1, 6.0));
        groups[0].tile.tile.texels = vec![Texel::default(); SIZE * SIZE];
        groups[0].tile.tile.texels[MIDDLE * SIZE + MIDDLE] = Texel { rgb: [0.2; 3], opacity: 0.5 };
        let mut image = RegionTileCache::default();
        assert_eq!(image.content_radius, None);
        image.merge(&groups);
        assert_eq!(image.content_radius, Some((1.5_f64 * 1.5 * 2.0).sqrt()));
        assert_eq!(image.bytes(), SIZE * SIZE * Texel::SIZE + SIZE * 8 + std::mem::size_of::<Option<f64>>()
            + std::mem::size_of::<Option<crate::model::TileFilterSummary>>()
            + (SIZE / crate::constants::TILE_FILTER_BLOCK_EDGE).pow(2) * 32);

        // Unchanged keys retain the image and its bound; changed contents rebuild both together.
        let previous = image.clone();
        assert!(image.is_current(&groups));
        groups[0].tile.key.as_mut().unwrap().simulated_at_ms += 1;
        groups[0].tile.tile.texels[SIZE - 1] = Texel { rgb: [0.1; 3], opacity: 0.5 };
        assert!(!image.is_current(&groups));
        image.merge(&groups);
        assert!(image.is_current(&groups));
        assert!(image.content_radius.unwrap() > previous.content_radius.unwrap());
        assert!(image.filter_summary.as_ref().unwrap().rgba_sum[3] > previous.filter_summary.as_ref().unwrap().rgba_sum[3]);

        // Removing the outer light must shrink the bound even though the tile dimensions are unchanged.
        groups[0].tile.key.as_mut().unwrap().simulated_at_ms += 1;
        groups[0].tile.tile.texels[SIZE - 1] = Texel::default();
        image.merge(&groups);
        assert_eq!(image.content_radius, previous.content_radius);
        assert_eq!(image.filter_summary, previous.filter_summary);
        groups[0].tile.tile.texels.fill(Texel::default());
        groups[0].tile.key.as_mut().unwrap().simulated_at_ms += 1;
        image.merge(&groups);
        assert_eq!(image.content_radius, Some(0.0));
        assert_eq!(image.filter_summary.as_ref().unwrap().rgba_sum, [0.0; 4]);
        assert!(image.lit_rows().iter().all(|&bits| bits == 0));

        // The canonical empty group releases the populated image payload and clears derived support.
        groups[0] = StarGroup::default();
        image.merge(&groups);
        assert_eq!(image.content_radius, Some(0.0));
        assert!(image.is_current(&groups) && image.is_empty());
        assert!(image.filter_summary.is_none());
        assert_eq!(image.bytes(), 0);
    }
}
