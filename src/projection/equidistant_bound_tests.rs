//! The equidistant cap rings: they hold the whole cap with its half-pixel margin, draw exactly the bits of the
//! annulus bound they narrow, and leave out a large share of its candidates.
use super::*;
use super::tests::{facing, observation_at};
use crate::constants::GRID_DEPTH;
use crate::model::{ViewCenter, hash_direction, region_caps};
use ProjectionKind::Equidistant;

#[test]
fn cap_rings_hold_every_point_of_the_cap_and_its_half_pixel_margin() {
    // Caps near and far from the centre, small and large, at every position angle; screens of 382 pixels per radian.
    let pixels_per_radian = 382.0;
    let mut checked = 0;
    for (angle, reach) in [(0.06_f64, 0.05_f64), (0.2, 0.05), (0.9, 0.05), (1.5, 0.05), (2.8, 0.3), (0.5, 0.45), (1.2, 0.02)] {
        for around in [0.0, 0.4, FRAC_PI_2, 2.0, PI, -2.5] {
            let (inner, outer) = (((angle - reach) * pixels_per_radian - 0.5).max(0.0), (angle + reach) * pixels_per_radian + 0.5);
            let spread = (reach.sin() / angle.sin()).min(1.0).asin();
            let annulus = Annulus { inner, outer, left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 };    // the box is not used by the rings
            let rings = find_equidistant_cap_rings(angle, reach, around, spread, pixels_per_radian, annulus);

            // Every direction of the cap, moved half a pixel each way on the screen (a hair less, so that rounding
            // cannot put a point exactly on a ring's edge just outside), lies in its row's columns.
            for step in 0..=200 {
                let theta = angle - reach + 2.0 * reach * step as f64 / 200.0;
                for turn in -200..=200 {
                    let phi = around + spread * turn as f64 / 200.0;
                    let distance = (theta.cos() * angle.cos() + theta.sin() * angle.sin() * (phi - around).cos()).clamp(-1.0, 1.0).acos();
                    if distance > reach { continue; }
                    let point = [theta * pixels_per_radian * phi.cos(), theta * pixels_per_radian * phi.sin()];
                    for offset in 0..8 {
                        let direction = offset as f64 * PI / 4.0;
                        let [x, y] = [point[0] + 0.4999 * direction.cos(), point[1] + 0.4999 * direction.sin()];
                        let columns = find_cap_ring_columns(&rings, y);
                        assert!(columns.is_some_and(|(left, right)| left <= x && x <= right),
                            "angle={angle} reach={reach} around={around}: ({x}, {y}) outside {columns:?}");
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 100_000, "{checked}");
}

#[test]
fn cap_rings_draw_the_bits_of_their_annulus_with_far_fewer_candidates() {
    // A dense tile of varied colours, drawn through the rings and through the annulus they narrow.
    let texels: Vec<Texel> = (0..TILE_EDGE_TEXELS.pow(2)).map(|index| {
        let opacity = 0.1 + (index % 37) as f32 / 50.0;
        Texel { rgb: [opacity * 0.3, opacity * 0.5, opacity * 0.7], opacity }
    }).collect();
    let mut sampler = TileSampler::new(TILE_EDGE_TEXELS, &Default::default());
    sampler.prepare(&[u64::MAX; TILE_EDGE_TEXELS], None);
    let size = 201;
    let (mut compared, mut ring_candidates, mut annulus_candidates) = (0, 0, 0);
    for fov in [10.0, 60.0, 120.0, 180.0, 300.0, 360.0] {
        for atmosphere in [false, true] {
            for center in [facing(5.83, 0.1), ViewCenter::Zenith, facing(2.0, -0.4)] {
                let (view, observation) = observation_at(fov, atmosphere, center, Equidistant);
                let screen = Screen::new(prepare_camera(&view), size);
                let mut regions: Vec<_> = (0..region_caps().len()).step_by(41).collect();
                regions.push(hash_direction(GRID_DEPTH, observation.patch.center));
                for region in regions {
                    let tile = TileGeometry::new(region);
                    let Some(mapping) = RegionMapping::new(&tile, &observation, &screen, None) else { continue };
                    let Image::CapRings { annulus, .. } = mapping.image else { continue };
                    let coarse = RegionMapping { image: Image::Annulus(annulus), ..mapping };

                    // Same pixels painted with the same bits, since every row still steps from the annulus start.
                    let mut narrowed = vec![Texel::default(); size * size];
                    let mut reference = vec![Texel::default(); size * size];
                    draw_region::<false>(&mut narrowed, &mut TouchedBlocks::default(), &mapping, &sampler, &texels);
                    draw_region::<false>(&mut reference, &mut TouchedBlocks::default(), &coarse, &sampler, &texels);
                    let bits = |pixels: &[Texel]| pixels.iter().map(|texel| [texel.rgb[0].to_bits(), texel.rgb[1].to_bits(), texel.rgb[2].to_bits(), texel.opacity.to_bits()]).collect::<Vec<_>>();
                    assert!(bits(&narrowed) == bits(&reference), "fov={fov} atmosphere={atmosphere} {center:?} region={region}: the rings changed the image");

                    // Count the candidates both bounds hand to the pixel loop.
                    let (top, bottom) = mapping.rows();
                    for row in top..=bottom {
                        let length = |runs: [Option<(usize, usize)>; 2]| runs.into_iter().flatten().map(|(first, last)| last - first + 1).sum::<usize>();
                        ring_candidates += length(mapping.columns_of(row));
                        annulus_candidates += length(coarse.columns_of(row));
                    }
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 100, "{compared}");
    assert!((ring_candidates as f64) < 0.8 * annulus_candidates as f64, "{ring_candidates} ring candidates against {annulus_candidates} in the annulus");
}
