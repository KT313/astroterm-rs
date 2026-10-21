//! The batched general mapping: every lane equals the single-pixel mapping bit for bit, including rejected,
//! singular and invalid rays, and batched spans paint exactly what single pixels paint at every span length.
use super::*;
use super::tests::{facing, observation_at};
use crate::model::{ViewCenter, region_caps};
use ProjectionKind::{Equidistant, Stereographic};

/// The views that take the general path: stereographic with refraction, equidistant with and without it.
fn general_views() -> [(f64, bool, ViewCenter, ProjectionKind); 6] {
    [(60.0, true, facing(5.83, 0.3), Stereographic), (180.0, true, ViewCenter::Zenith, Stereographic), (359.0, true, facing(2.0, 0.5), Stereographic),
        (10.0, true, facing(5.83, 0.0), Equidistant), (180.0, false, ViewCenter::Zenith, Equidistant), (360.0, true, facing(1.0, -0.2), Equidistant)]
}

/// The result's bits, so that equal results compare equal whatever their values.
fn bits_of(result: Option<(Point, f64)>) -> Option<[u64; 3]> {
    result.map(|(position, footprint)| [position[0].to_bits(), position[1].to_bits(), footprint.to_bits()])
}

/// Map `rays` in every lane position of a batch, the other lanes holding the neighbouring rays, and compare each
/// with the single-pixel mapping. Returns how many lanes were kept and rejected.
fn compare_batches(mapping: &RegionMapping<'_>, rays: &[(Vector3, f64, f64)], context: &str) -> [usize; 2] {
    let mut counts = [0; 2];
    for start in 0..rays.len() {
        let batch: [(Vector3, f64, f64); PROJECTION_MAPPING_BATCH_PIXELS] = std::array::from_fn(|lane| rays[(start + lane) % rays.len()]);
        for (lane, (&(numerator, spread, factor), batched)) in batch.iter().zip(mapping.tile_of_overlapping_footprints(&batch)).enumerate() {
            let single = mapping.tile_of_overlapping_footprint(numerator, spread, factor);
            assert_eq!(bits_of(batched), bits_of(single), "{context}, lane {lane}: ray {:?}", batch[lane]);
            counts[single.is_some() as usize] += 1;
        }
    }
    counts
}

#[test]
fn batched_rays_map_to_the_bits_of_single_pixels() {
    // Rays as the span loop steps them, across regions, rows and both sides of every tile edge.
    let size = 601;
    let mut counts = [0; 2];
    for (fov, atmosphere, center, projection) in general_views() {
        let (view, observation) = observation_at(fov, atmosphere, center, projection);
        let screen = Screen::new(prepare_camera(&view), size);
        for region in (0..region_caps().len()).step_by(29) {
            let tile = TileGeometry::new(region);
            let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue };
            let (top, bottom) = mapping.rows();
            for row in (top..=bottom).step_by(3) {
                for span in mapping.spans_of(row).into_iter().flatten() {
                    let mut rays = RowRays::new(&screen, span.origin as f64, row as f64);
                    let stepped: Vec<_> = (span.origin..=span.last).map(|_| { let ray = rays.current(&screen); rays.advance(); ray }).collect();
                    for (chunk, batch) in stepped.chunks(PROJECTION_MAPPING_BATCH_PIXELS).enumerate() {
                        let context = format!("fov={fov} {projection:?} atmosphere={atmosphere} region={region} row={row} chunk={chunk}");
                        let [rejected, kept] = compare_batches(&mapping, batch, &context);
                        counts[0] += rejected;
                        counts[1] += kept;
                    }
                }
            }
        }
    }
    assert!(counts[0] > 10_000 && counts[1] > 10_000, "rejected and kept lanes: {counts:?}");
}

#[test]
fn singular_and_invalid_rays_are_rejected_like_single_pixels() {
    // The point opposite the tile centre, zero, NaN and infinite spreads and components, mixed with ordinary rays.
    let mut compared = 0;
    for (fov, atmosphere, center, projection) in general_views() {
        let (view, observation) = observation_at(fov, atmosphere, center, projection);
        let screen = Screen::new(prepare_camera(&view), 301);
        for region in (0..region_caps().len()).step_by(211) {
            let tile = TileGeometry::new(region);
            let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue };
            let [east, north, centre] = mapping.to_tile.0.map(|[x, y, z]| Vector3 { x, y, z });
            let opposite = Vector3::default() - centre;                                        // the tile axes are orthonormal, so this row is the centre in horizon axes
            let ordinary = screen.ray(150.0, 150.0);
            let rays = [
                ordinary, (opposite, 1.0, 1.0), (opposite * 2.0, 2.0, 1.0), (centre, 1.0, 1.0), (east, 1.0, 1.0), (north * -1.0, 1.0, 1.0),
                (ordinary.0, 0.0, 1.0), (ordinary.0, f64::NAN, 1.0), (ordinary.0, f64::INFINITY, 1.0), (ordinary.0, -1.0, 1.0),
                (Vector3 { x: f64::NAN, ..ordinary.0 }, ordinary.1, ordinary.2), (Vector3 { z: f64::INFINITY, ..ordinary.0 }, ordinary.1, ordinary.2),
                (ordinary.0 * 1e300, ordinary.1 * 1e300, ordinary.2), (Vector3::default(), 1.0, 1.0), (ordinary.0, ordinary.1, f64::NAN), (ordinary.0, ordinary.1, 1e300),
            ];
            compare_batches(&mapping, &rays, &format!("fov={fov} {projection:?} atmosphere={atmosphere} region={region}"));
            compared += 1;
        }
    }
    assert!(compared > 20, "{compared}");
}

#[test]
fn batched_spans_paint_like_single_pixels_at_every_length() {
    // A dense tile of varied colours, so that nearly every kept pixel paints.
    let texels: Vec<Texel> = (0..TILE_EDGE_TEXELS.pow(2)).map(|index| {
        let opacity = 0.1 + (index % 37) as f32 / 50.0;
        Texel { rgb: [opacity * 0.3, opacity * 0.5, opacity * 0.7], opacity }
    }).collect();
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS, &Default::default());
    sampler.prepare(&[u64::MAX; TILE_EDGE_TEXELS], None);

    // An empty span writes nothing.
    let (view, observation) = observation_at(180.0, true, ViewCenter::Zenith, Stereographic);
    let screen = Screen::new(prepare_camera(&view), 301);
    let mapping = (0..region_caps().len()).find_map(|region| RegionMapping::new(&TileGeometry::new(region), &observation, &screen, None)).unwrap();
    assert_eq!(draw_screen_span::<true>(&mut [], &mapping, &sampler, &texels, Span { origin: 3, first: 5, last: 5 }, 150), (usize::MAX, 0));

    // Each span whole and cut to 1–9 pixels, at its own start and two pixels further in.
    let size = 301;
    let mut painted = 0;
    for (fov, atmosphere, center, projection) in general_views() {
        let (view, observation) = observation_at(fov, atmosphere, center, projection);
        let screen = Screen::new(prepare_camera(&view), size);
        for region in (0..region_caps().len()).step_by(53) {
            let tile = TileGeometry::new(region);
            let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue };
            let (top, bottom) = mapping.rows();
            for row in (top..=bottom).step_by(5) {
                for span in mapping.spans_of(row).into_iter().flatten() {
                    for skip in [0, 2] {
                        let first = span.first + skip;
                        for last in (first..first + 9).chain([span.last]).filter(|&last| last <= span.last && last >= first) {
                            let cut = Span { origin: span.origin, first, last };
                            let mut batched = vec![Texel::default(); last - first + 1];
                            let mut single = batched.clone();
                            let written = draw_screen_span::<true>(&mut batched, &mapping, &sampler, &texels, cut, row);
                            let expected = draw_single_pixels(&mut single, &mapping, &sampler, &texels, cut, row);
                            let bits = |pixels: &[Texel]| pixels.iter().map(|texel| [texel.rgb[0].to_bits(), texel.rgb[1].to_bits(), texel.rgb[2].to_bits(), texel.opacity.to_bits()]).collect::<Vec<_>>();
                            assert!(bits(&batched) == bits(&single), "fov={fov} {projection:?} region={region} row={row} pixels {first}..={last}");
                            assert_eq!(written, expected, "fov={fov} {projection:?} region={region} row={row} pixels {first}..={last}");
                            painted += single.iter().filter(|texel| texel.opacity > 0.0).count();
                        }
                    }
                }
            }
        }
    }
    assert!(painted > 20_000, "{painted}");
}

/// The span loop before batching: map, sample and blend one pixel at a time, recording the written columns.
fn draw_single_pixels(pixels: &mut [Texel], mapping: &RegionMapping<'_>, sampler: &TileSampler<'_>, texels: &[Texel], span: Span, row: usize) -> (usize, usize) {
    let mut written = (usize::MAX, 0);
    let mut rays = RowRays::new(mapping.screen, span.origin as f64, row as f64);
    for _ in span.origin..span.first { rays.advance(); }
    for (column, pixel) in (span.first..).zip(pixels) {
        let (numerator, spread, factor) = rays.current(mapping.screen);
        rays.advance();
        let Some((position, footprint)) = mapping.tile_of_overlapping_footprint(numerator, spread, factor) else { continue };
        if sample_and_blend_pixel(pixel, sampler, texels, position, footprint) {
            written = (written.0.min(column), column);
        }
    }
    written
}
