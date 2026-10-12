//! The image caches of plan section 4: one sky-fixed tile per group (the group's brightness prefix drawn as the
//! screen would draw it) and one per region (its group images added together). Tone mapping and the zoom boost
//! happen in the frame output, not here.
use crate::astro::Vector3;
use crate::constants::{SPEED_GROUP_COUNT, TILE_EDGE_TEXELS};
use crate::regions::appearance::{StarOpacities, Texel, blend_star, order_star_rows, premultiplied_color};
use crate::regions::tiles::TileGeometry;
use crate::model::GroupTable;

/// What a group image was rendered from. Two renders with equal keys are identical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageKey {
    pub epoch: f64,                         // simulated epoch (TT) of the rows drawn
    pub rows: usize,                        // the brightness prefix drawn (the threshold and the computational interval decide it)
    pub threshold: u16,                     // magnitude threshold as a catalog code; a row is drawn only when its current code is at most this
}

impl ImageKey {
    /// The key of an image whose prefix is empty: nothing to draw, whatever the threshold.
    pub const EMPTY: ImageKey = ImageKey { epoch: 0.0, rows: 0, threshold: 0 };
}

/// The image cache of one group (plan section 4, "Group image"): the group's brightness prefix at its simulation
/// epoch, drawn into the region's fixed-size tile.
///
/// **Key:** `ImageKey`: the simulation epoch, the prefix length and the magnitude threshold. **Invalidated by:**
/// any key change; the image is then drawn again from the simulation cache. Rotation, panning, zoom, resize and
/// simulated time inside the group's hold change no key and so re-render nothing. A group whose prefix is empty
/// holds no texels. Images are never dropped: a region that leaves the view keeps its images for its return.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupImage {
    key: Option<ImageKey>,
    texels: Vec<Texel>,                     // `TileGeometry::texel_count()` entries, or none for an empty prefix
}

impl GroupImage {
    pub fn key(&self) -> Option<ImageKey> { self.key }
    pub fn texels(&self) -> &[Texel] { &self.texels }
    pub fn is_empty(&self) -> bool { self.texels.is_empty() }
    pub fn bytes(&self) -> usize { self.texels.len() * Texel::SIZE }

    /// Draw rows `0..key.rows` of the simulation (`directions` and `magnitudes` at `key.epoch`) into the tile: each
    /// row brighter than the threshold becomes the four-texel glyph of its colour, blended source-over from dim
    /// to bright current magnitude, then ascending ID. Catalog brightness-bound order is unchanged. Rows the model
    /// could not place and rows outside the tile are skipped.
    pub fn render(&mut self, key: ImageKey, tile: &TileGeometry, table: &GroupTable, directions: &[Vector3], magnitudes: &[u16], opacities: &StarOpacities) {
        self.key = Some(key);
        if key.rows == 0 { self.texels = Vec::new(); return; }
        let size = TileGeometry::size();
        self.texels.clear();
        self.texels.resize(TileGeometry::texel_count(), Texel::default());
        let columns = table.columns();
        let rows = order_star_rows(&magnitudes[..key.rows], &columns.id[..key.rows], key.threshold);
        for row in rows {
            let (direction, code) = (directions[row], magnitudes[row]);
            let Some((column, line)) = tile.project(direction) else { continue };
            let (x, y) = (round_half_away(column), round_half_away(line));
            if x < 1 || y < 1 || x >= size as i64 || y >= size as i64 { continue; }            // the four-texel footprint must fit, as on the screen
            let opacity = opacities.of(code);
            let premultiplied = premultiplied_color(columns.display_color[row], opacity);
            let bottom_right = y as usize * size + x as usize;
            for offset in [bottom_right - size - 1, bottom_right - size, bottom_right - 1, bottom_right] {
                blend_star(&mut self.texels[offset], premultiplied, opacity);
            }
        }
    }
}

/// `v.round() as i64` without the library call, as the screen projection rounds.
#[inline]
fn round_half_away(v: f64) -> i64 { (v + 0.5_f64.copysign(v)) as i64 }

/// The image cache of one region (plan section 4, "Region image"): its group images added together, texel by texel.
///
/// **Key:** the keys of the region's group images. **Invalidated by:** any group image of the region being
/// rendered again (its key changed); the sum is then rebuilt from the group images. Starlight adds, so the
/// stacking order of the groups is irrelevant. Never dropped, like the group images.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionImage {
    keys: Option<[Option<ImageKey>; SPEED_GROUP_COUNT]>, // the group keys as summed; None before the first sum
    texels: Vec<Texel>,                     // the tile, or none when every group image is empty
    lit: Vec<u64>,                          // per tile row, bit x set when texel x of that row holds light; lets the warp skip empty areas
}

impl RegionImage {
    pub fn texels(&self) -> &[Texel] { &self.texels }
    pub fn is_empty(&self) -> bool { self.texels.is_empty() }
    pub fn bytes(&self) -> usize { self.texels.len() * Texel::SIZE }
    pub fn is_summed(&self) -> bool { self.keys.is_some() }
    /// One bit per texel, a word per tile row: which texels hold light. Empty when the image is.
    pub fn lit_rows(&self) -> &[u64] { &self.lit }

    /// Whether the sum still matches the group images.
    pub fn is_current(&self, groups: &[GroupImage]) -> bool {
        self.keys.as_ref().is_some_and(|keys| keys.iter().zip(groups).all(|(key, group)| *key == group.key()))
    }

    /// Rebuild the sum of `groups`.
    pub fn sum(&mut self, groups: &[GroupImage]) {
        self.keys = Some(std::array::from_fn(|group| groups[group].key()));
        let mut parts = groups.iter().filter(|group| !group.is_empty());
        let Some(first) = parts.next() else { self.texels = Vec::new(); self.lit = Vec::new(); return; };
        self.texels.clear();
        self.texels.extend_from_slice(first.texels());
        for part in parts {
            for (sum, texel) in self.texels.iter_mut().zip(part.texels()) { *sum = sum.plus(*texel); }
        }
        self.lit.clear();
        self.lit.extend(self.texels.chunks_exact(TILE_EDGE_TEXELS).map(|row| row.iter().enumerate().filter(|(_, texel)| !texel.is_empty()).fold(0_u64, |bits, (x, _)| bits | 1 << x)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::J2000;
    use crate::constants::{GRID_DEPTH, STAR_OPACITY_MAGNITUDE_SCALE, STAR_OPACITY_REFERENCE_MAGNITUDE};
    use crate::catalog::magnitude_code;
    use crate::model::{StarColor, StarRow, hash_direction};
    use crate::model::fixtures;
    use crate::regions::appearance::star_opacity;
    use crate::regions::UNPLACED;

    const SIZE: usize = 64;
    const MIDDLE: usize = 32;                                                               // the region centre at 31.5 rounds up: its glyph covers texels 31 and 32

    fn region() -> usize { hash_direction(GRID_DEPTH, Vector3 { x: 1.0, y: 0.0, z: 0.0 }) }
    fn table(rows: &[StarRow]) -> GroupTable {
        let stars = fixtures::layout(&rows.iter().map(|row| (region(), 0, *row)).collect::<Vec<_>>());
        stars.group(region(), 0).clone()
    }
    fn key(rows: usize, threshold: f64) -> ImageKey { ImageKey { epoch: J2000, rows, threshold: magnitude_code(threshold) } }
    fn opacities() -> StarOpacities { let mut table = StarOpacities::default(); table.prepare(1.0); table }
    fn lit(texels: &[Texel]) -> Vec<(usize, Texel)> { texels.iter().copied().enumerate().filter(|(_, texel)| !texel.is_empty()).collect() }

    #[test]
    fn a_star_at_the_region_centre_lights_the_four_texels_around_the_middle_with_its_colour_and_opacity() {
        let tile = TileGeometry::new(region());
        let mut row = fixtures::row(tile.center(), 0.0, 3.0, 7);
        row.display_color = StarColor::RedOrange.index();
        let table = table(&[row]);
        let mut image = GroupImage::default();
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
        let mut image = GroupImage::default();
        image.render(key(5, 6.0), &tile, &table, &directions, &codes, &opacities());
        assert_eq!(lit(image.texels()).len(), 8);                                            // the centre star and the inside star; the rest skipped
        image.render(key(1, 6.0), &tile, &table, &directions, &codes, &opacities());
        assert_eq!(lit(image.texels()).len(), 4);                                            // a shorter prefix
        image.render(key(5, 2.0), &tile, &table, &directions, &codes, &opacities());
        assert!(image.texels().iter().all(|texel| texel.is_empty()));                       // a brighter threshold: everything too faint, the tile still allocated
        image.render(ImageKey::EMPTY, &tile, &table, &directions, &codes, &opacities());
        assert!(image.is_empty() && image.bytes() == 0);                                     // an empty prefix holds no texels
    }

    #[test]
    fn stars_in_one_image_blend_source_over_as_on_the_screen() {
        let tile = TileGeometry::new(region());
        let faint = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;  // opacity 0.1
        let table = table(&[fixtures::row(tile.center(), 0.0, faint, 1), fixtures::row(tile.center(), 0.0, faint, 2)]);
        let mut image = GroupImage::default();
        image.render(key(2, 9.0), &tile, &table, &[tile.center(); 2], &[magnitude_code(faint); 2], &opacities());
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
            let mut image = GroupImage::default();
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
    fn the_region_image_is_the_sum_of_its_group_images_and_tracks_their_keys() {
        let tile = TileGeometry::new(region());
        let faint = STAR_OPACITY_REFERENCE_MAGNITUDE + 1.0 / STAR_OPACITY_MAGNITUDE_SCALE;
        let table = table(&[fixtures::row(tile.center(), 0.0, faint, 1)]);
        let mut groups = vec![GroupImage::default(), GroupImage::default(), GroupImage::default()];
        groups[0].render(key(1, 9.0), &tile, &table, &[tile.center()], &[magnitude_code(faint)], &opacities());
        groups[2].render(key(1, 9.0), &tile, &table, &[tile.center()], &[magnitude_code(faint)], &opacities());
        let mut region_image = RegionImage::default();
        assert!(!region_image.is_current(&groups) && !region_image.is_summed());
        region_image.sum(&groups);
        assert!(region_image.is_current(&groups) && region_image.is_summed());
        let middle = MIDDLE * SIZE + MIDDLE;
        assert!((region_image.texels()[middle].opacity - 0.2).abs() < 1e-3);                 // added, not blended: 0.1 + 0.1
        assert_eq!(region_image.texels()[middle].rgb, groups[0].texels()[middle].rgb.map(|c| c * 2.0));
        let lit: Vec<(usize, u32)> = region_image.lit_rows().iter().enumerate().filter(|(_, bits)| **bits != 0).map(|(y, bits)| (y, bits.count_ones())).collect();
        assert_eq!(lit, [(MIDDLE - 1, 2), (MIDDLE, 2)]);                                     // the glyph's two rows, two texels each
        assert_eq!(region_image.lit_rows()[MIDDLE] & (1 << MIDDLE), 1 << MIDDLE);
        let mut reversed = RegionImage::default();
        reversed.sum(&[groups[2].clone(), groups[1].clone(), groups[0].clone()]);
        assert_eq!(reversed.texels(), region_image.texels());                               // stacking order is irrelevant
        groups[2].render(key(1, 2.0), &tile, &table, &[tile.center()], &[magnitude_code(faint)], &opacities());
        assert!(!region_image.is_current(&groups));                                          // a group key changed
        region_image.sum(&groups);
        assert!((region_image.texels()[middle].opacity - 0.1).abs() < 1e-3);
        region_image.sum(&[GroupImage::default(), GroupImage::default(), GroupImage::default()]);
        assert!(region_image.is_empty() && region_image.bytes() == 0 && region_image.is_summed() && region_image.lit_rows().is_empty()); // only empty groups: no texels
    }
}
