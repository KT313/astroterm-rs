//! Borrow validated arrays once per processing pass. No ownership, validation, precision or cache-format changes.
use super::{StarStorage, decode_motion, expand};
use crate::{
    astro::{Vector3, models::stars::StellarMotion},
    catalog::{EncodedDesignation, NameId},
    model::ObservedStar,
};

pub(crate) struct ObservationFields<'a> {
    magnitude: &'a [f32],
}
impl ObservationFields<'_> {
    pub fn create_observed_star(&self, index: usize, drawable: bool) -> ObservedStar {
        ObservedStar {
            source_index: index,
            drawable,
            magnitude: f64::from(self.magnitude[index]),
            position: Vector3::default(),
        }
    }
}
pub(crate) struct TrajectoryFields<'a> {
    u0: [&'a [f32]; 3],
    w: [&'a [f32]; 3],
    distance: &'a [f32],
    precise_indices: &'a [u32],
    precise_motions: &'a [[f64; 7]],
}
impl TrajectoryFields<'_> {
    pub fn motion(&self, index: usize) -> StellarMotion {
        let precise = self.precise_indices[index];
        if precise != 0 {
            return decode_motion(self.precise_motions[precise as usize - 1]);
        }
        StellarMotion {
            u0: expand(std::array::from_fn(|axis| self.u0[axis][index])),
            w: expand(std::array::from_fn(|axis| self.w[axis][index])),
            distance_pc: (self.distance[index] > 0.0).then_some(f64::from(self.distance[index])),
        }
    }
}
impl StarStorage {
    pub(crate) fn borrow_observation_fields(&self) -> ObservationFields<'_> {
        ObservationFields {
            magnitude: &self.magnitude,
        }
    }
    pub fn name(&self, index: usize) -> Option<NameId> {
        let name = self.names[index];
        (name != 0).then(|| NameId::from_range(self.name_table[name as usize - 1]))
    }
    pub fn designation(&self, index: usize) -> EncodedDesignation {
        EncodedDesignation::from_validated_bytes(self.designations[index])
    }
    pub fn spectral_type(&self, index: usize) -> [u8; 2] {
        self.spectral_types[index]
    }
    pub fn color_index(&self, index: usize) -> Option<f32> {
        (self.flags[index] & 2 != 0).then_some(self.colors[index])
    }

    pub(crate) fn borrow_trajectory_fields(&self) -> TrajectoryFields<'_> {
        TrajectoryFields {
            u0: std::array::from_fn(|axis| &*self.u0[axis]),
            w: std::array::from_fn(|axis| &*self.w[axis]),
            distance: &self.distance,
            precise_indices: &self.precise_indices,
            precise_motions: &self.precise_motions,
        }
    }
    pub(crate) fn brightness_keys(&self) -> &[f32] {
        &self.brightness_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{catalog_fingerprint, load_cached_catalog, write_cached_catalog};

    #[test]
    fn borrowed_owned_and_mapped_fields_match_full_records() {
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
        let owned = crate::sky::prepare_owned_catalog(parsed);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog");
        let fingerprint = catalog_fingerprint();
        write_cached_catalog(&path, &owned, &fingerprint).unwrap();
        let mapped = load_cached_catalog(&path, &fingerprint).unwrap();
        for catalog in [&owned, &mapped] {
            let fields = catalog.stars.borrow_observation_fields();
            let trajectories = catalog.stars.borrow_trajectory_fields();
            for index in 0..catalog.stars.len() {
                let full = catalog.stars.get(index);
                let observed = fields.create_observed_star(index, true);
                assert_eq!(observed, ObservedStar::from_star(&full, index, Vector3::default()));
                let view = crate::model::ObservedStarView {
                    state: &observed,
                    catalog: &catalog.stars,
                };
                assert!(std::ptr::eq(view.state, &observed));
                assert!(std::ptr::eq(view.catalog, &catalog.stars));
                assert_eq!(view.id(), full.id);
                assert_eq!(view.name(), full.name);
                assert_eq!(view.spectral_type(), full.spectral_type);
                assert_eq!(view.color_index(), full.color_index);
                assert_eq!(view.has_data(), full.has_data);
                assert_eq!(view.designation().resolve(), full.designation);
                assert_eq!(
                    catalog.names.get(catalog.stars.name(index)),
                    catalog.names.get(full.name)
                );
                assert_eq!(trajectories.motion(index), catalog.stars.motion(index));
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
            crate::sky::load_sky_catalog(dataset.as_ref(), &directories, &mut std::io::stderr()).unwrap();
        let indices: Vec<_> = catalog
            .stars
            .brightness_keys()
            .iter()
            .enumerate()
            .filter(|(_, m)| **m <= 10.0)
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
                                s.designation,
                                s.magnitude,
                                s.spectral_type,
                                s.color_index,
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
