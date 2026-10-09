//! Borrow validated columns once per processing pass. The vector columns get typed N×3 views; scalar columns
//! are plain slices already. The owner keeps the complete allocated columns; these views are processing inputs.
use super::{StarStorage, expand};
use crate::{
    astro::models::stars::StellarMotion,
    catalog::{NameId},

};
use ndarray::ArrayView2;
#[cfg(test)]
use crate::{model::ObservedStar, astro::Vector3};

#[cfg(test)]
pub(crate) struct ObservationFields<'a> {
    magnitude: &'a [u16],
}
#[cfg(test)]
impl ObservationFields<'_> {
    pub fn create_observed_star(&self, index: usize, drawable: bool) -> ObservedStar {
        ObservedStar {
            source_index: index,
            drawable,
            magnitude: crate::catalog::decode_magnitude(self.magnitude[index]),
            position: Vector3::default(),
        }
    }
}
pub(crate) struct TrajectoryFields<'a> {
    u0: &'a [[f32; 3]],
    w: &'a [[f32; 3]],
    distance: &'a [f32],
}
impl TrajectoryFields<'_> {
    pub fn motion(&self, index: usize) -> StellarMotion {
        StellarMotion {
            u0: expand(self.u0[index]),
            w: expand(self.w[index]),
            distance_pc: (self.distance[index] > 0.0).then_some(f64::from(self.distance[index])),
        }
    }
}
impl StarStorage {
    /// Stored unit directions as an N×3 view, one row per catalog index; zero copy for source and cache-loaded data.
    pub fn directions(&self) -> ArrayView2<'_, f32> {
        ArrayView2::from(self.rows.u0.as_slice())
    }
    /// Normalized motion per Julian year as an N×3 view; rows with a precise entry are zero.
    pub fn motions(&self) -> ArrayView2<'_, f32> {
        ArrayView2::from(self.rows.w.as_slice())
    }
    pub(crate) fn brightness_keys(&self) -> &[u16] {
        self.rows.brightness_key.as_slice()
    }
    #[cfg(test)]
    pub(crate) fn borrow_observation_fields(&self) -> ObservationFields<'_> {
        ObservationFields {
            magnitude: self.rows.magnitude.as_slice(),
        }
    }
    pub(crate) fn borrow_trajectory_fields(&self) -> TrajectoryFields<'_> {
        let c = self.columns();
        TrajectoryFields {
            u0: c.u0,
            w: c.w,
            distance: c.distance,
        }
    }
    pub fn name(&self, index: usize) -> Option<NameId> {
        let name = self.rows.name.as_slice()[index];
        NameId::from_entry(name)
    }
    pub fn display_color(&self, index: usize) -> crate::model::StarColor {
        crate::model::StarColor::from_index(self.rows.display_color.as_slice()[index]).expect("validated color index")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{catalog_fingerprint, load_cached_catalog, write_cached_catalog};

    #[test]
    fn borrowed_source_and_cached_fields_match_full_records() {
        let mut parsed = crate::catalog::load_embedded_catalog().unwrap();
        parsed.stars[0].space_motion = Some(crate::catalog::SpaceMotion {
            distance_pc: 1.0,
            position: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            velocity: Vector3 {
                x: 0.0,
                y: 1e-7,
                z: 0.0,
            },
        });
        let owned = crate::sky::prepare_owned_catalog(parsed).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog");
        let fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &owned, &fingerprint).unwrap();
        let cached = load_cached_catalog(&path, &fingerprint).unwrap();
        for catalog in [&owned.catalog, &cached.catalog] {
            let fields = catalog.stars.borrow_observation_fields();
            let trajectories = catalog.stars.borrow_trajectory_fields();
            let directions = catalog.stars.directions();
            assert_eq!(directions.shape(), &[catalog.stars.len(), 3]);
            for index in 0..catalog.stars.len() {
                let full = catalog.stars.get(index);
                let observed = fields.create_observed_star(index, true);
                assert_eq!(observed, ObservedStar::from_star(&full, index, Vector3::default()));
                let view = crate::model::ObservedStarView {
                    state: crate::model::ObservedStarState::Borrowed(&observed),
                    catalog: &catalog.stars,
                };
                assert!(std::ptr::eq(view.state.as_ref(), &observed));
                assert!(std::ptr::eq(view.catalog, &catalog.stars));
                assert_eq!(view.id(), full.id);
                assert_eq!(view.name(), full.name);
                assert_eq!(view.display_color(), full.display_color);
                assert_eq!(view.has_data(), full.has_data);
                assert_eq!(
                    catalog.names.get(catalog.stars.name(index)),
                    catalog.names.get(full.name)
                );
                assert_eq!(trajectories.motion(index), catalog.stars.motion(index));
                let row = directions.row(index);
                assert_eq!(expand([row[0], row[1], row[2]]), catalog.stars.stored_direction(index));
            }
        }
    }
}

#[cfg(test)]
mod measurements {
    use std::{hint::black_box, time::Instant};
    #[test]
    #[ignore = "release borrowed catalog access comparison; optional ASTROTERM_DATASET"]
    fn compare_catalog_access_paths() {
        use crate::catalog::datasets::{Dataset, DatasetDirectories};
        let dataset = std::env::var_os("ASTROTERM_DATASET").map(|p| Dataset::Path(p.into()));
        let directories = DatasetDirectories {
            data: None,
            cache: Some(std::env::temp_dir().join("astroterm-processing-probe")),
        };
        let catalog =
            crate::sky::load_sky_catalog(dataset.as_ref(), &directories, &mut std::io::stderr()).unwrap().catalog;
        let indices: Vec<_> = catalog
            .stars
            .brightness_keys()
            .iter()
            .enumerate()
            .filter(|(_, m)| crate::catalog::passes_brightness_bound(**m, 10.0))
            .map(|(i, _)| i)
            .take(500000)
            .collect();
        let fields = catalog.stars.borrow_observation_fields();
        let trajectories = catalog.stars.borrow_trajectory_fields();
        let mut totals = [0.0; 4];
        for iteration in 0..8 {
            for mode in if iteration % 2 == 0 { [0, 1, 2, 3] } else { [3, 2, 1, 0] } {
                let start = Instant::now();
                for &i in &indices {
                    match mode {
                        0 => {
                            let s = catalog.stars.get(black_box(i));
                            black_box((
                                s.id,
                                s.name,
                                s.magnitude,
                                s.display_color,
                                s.has_data,
                            ));
                        }
                        1 => {
                            black_box(fields.create_observed_star(black_box(i), true));
                        }
                        2 => {
                            black_box(catalog.stars.motion(black_box(i)));
                        }
                        _ => {
                            black_box(trajectories.motion(black_box(i)));
                        }
                    }
                }
                if iteration > 0 {
                    totals[mode] += start.elapsed().as_secs_f64() * 1000.0;
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({"selected":indices.len(),"frames":7,
            "full_metadata_ms":totals[0]/7.0,"borrowed_packed_metadata_ms":totals[1]/7.0,
            "individual_motion_access_ms":totals[2]/7.0,"borrowed_motion_access_ms":totals[3]/7.0})
        );
    }
}

/// Only columns required for intrinsic stellar motion and brightness; no names, IDs or display metadata.
pub(crate) struct StellarFields<'a> {
    trajectories: TrajectoryFields<'a>,
    magnitudes: &'a [u16],
}
impl StellarFields<'_> {
    pub fn motion(&self, index: usize) -> StellarMotion { self.trajectories.motion(index) }
    pub fn magnitude(&self, index: usize) -> f64 { crate::catalog::decode_magnitude(self.magnitudes[index]) }
}
impl StarStorage {
    pub(crate) fn borrow_stellar_fields(&self) -> StellarFields<'_> {
        StellarFields { trajectories: self.borrow_trajectory_fields(), magnitudes: self.columns().magnitude }
    }
}
