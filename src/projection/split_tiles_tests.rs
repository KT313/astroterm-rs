//! Stars of split regions on screen: drawn from the child tiles at their forward-projected pixels, and as small as
//! the finer tiles make them.
use super::*;
use super::tests::facing;
use crate::constants::{GRID_DEPTH, MIN_TILE_TEXELS_PER_PIXEL};
use crate::formulas::{J2000, Observer};
use crate::model::{fixtures, hash_direction};
use crate::observation::observe;
use crate::projection::{Frame, project_camera};
use crate::regions::refresh_regions;
use crate::selection::select_regions;
use ProjectionKind::{Equidistant, Stereographic};

#[test]
fn stars_of_split_regions_land_at_their_pixels_and_stay_small() {
    let time = crate::formulas::utc_jd_to_time_ms(J2000).unwrap();
    let site = Observer { latitude: 0.6, longitude: 2.4 };
    let size = 1116;
    for (fov, atmosphere, projection) in [(1.0, false, Stereographic), (5.0, true, Stereographic), (5.0, true, Equidistant), (10.0, false, Stereographic)] {
        let view = View { center: facing(1.0, 0.4), projection, fov_degrees: fov };
        let observation = observe(&site, atmosphere, &view, time, crate::formulas::evaluate_earth_state(time), &mut StepTimes::default());
        let camera = prepare_camera(&view);
        for offset in [0.0, 0.3] {
            // One star at or off the view centre, alone in its region.
            let centre = observation.patch.center;
            let across = crate::formulas::Vector3 { x: 0.0, y: 0.0, z: 1.0 }.cross(centre).normalized();
            let direction = (centre + across * (offset * (fov / 2.0_f64).to_radians())).normalized();
            let region = hash_direction(GRID_DEPTH, direction);
            let mut data = fixtures::layout(&[(region, 0, fixtures::row(direction, 0.0, 3.0, 1))]);

            // Steps 3 to 5, with the region split.
            let mut selected = Selected::default();
            select_regions(&mut selected, &data.star_regions, &observation, &view, size, &mut StepTimes::default());
            assert!(!selected.leaves.is_empty(), "fov={fov} {projection:?}: the region is not split");
            refresh_regions(
                &mut data.star_regions, &data.constellation_stars,
                &mut data.constellation_simulation, &mut data.constellation_last_update_ms,
                &mut data.star_opacities, &mut data.tile_bytes,
                &selected, time, view.fov_degrees, 6.0, &mut StepTimes::default(),
            );
            let mut frame = Frame::square(size);
            let projected_stars = project_test_image(size, &data.star_regions, &selected, &observation, &view, &mut StepTimes::default());
            frame.insert_projected_stars(&projected_stars);

            // The light's centre lies at the star's pixel within half a child texel, and its spread is a few child texels.
            let radius = (size as f64 - 1.0) * 0.5;
            let expected = project_camera(camera, observation.apparent_star_direction(direction)).unwrap();
            let expected = [radius + expected.x * radius, radius - expected.y * radius];
            let (mut total, mut centre, mut spread) = (0.0, [0.0; 2], 0.0);
            for (index, texel) in frame.stars().iter().enumerate() {
                let (x, y, weight) = ((index % size) as f64, (index / size) as f64, texel.opacity as f64);
                total += weight;
                centre = [centre[0] + x * weight, centre[1] + y * weight];
                spread += ((x - expected[0]).powi(2) + (y - expected[1]).powi(2)) * weight;
            }
            assert!(total > 0.0, "fov={fov} {projection:?} offset={offset}: no light");
            let pixels_per_texel = 1.0 / MIN_TILE_TEXELS_PER_PIXEL;                          // at most, in a split view
            let tolerance = 0.5 * pixels_per_texel + 1.0;
            assert!((centre[0] / total - expected[0]).abs() < tolerance && (centre[1] / total - expected[1]).abs() < tolerance,
                "fov={fov} {projection:?} offset={offset}: centre {:?}, expected {expected:?}", [centre[0] / total, centre[1] / total]);
            assert!((spread / total).sqrt() < 2.0 * pixels_per_texel, "fov={fov} {projection:?} offset={offset}: spread {} pixels", (spread / total).sqrt());
        }
    }
}
