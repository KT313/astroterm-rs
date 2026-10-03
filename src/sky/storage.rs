//! Immutable structure-of-arrays storage. Arithmetic is f64 after expanding the compact inputs; only a sparse
//! exception table retains trajectories whose certified quantization error would exceed half an arcsecond.
use super::Star;
use crate::astro::{
    Vector3,
    models::stars::{StellarMotion, computational_years},
};
use crate::catalog::{Designation, NameId, StarId};

/// The grid uses the effective stored trajectory, so this covers cell-direction rounding and f64 bound
/// arithmetic, not the original catalog's quantization error. Model error is certified separately below.
pub const QUANTIZATION_MARGIN: f64 = 0.1 * std::f64::consts::PI / (180.0 * 3600.0);
const MAX_DIRECTION_ERROR: f64 = 0.5 * std::f64::consts::PI / (180.0 * 3600.0);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StarStorage {
    u0: [Vec<f32>; 3],
    w: [Vec<f32>; 3],
    magnitude: Vec<f32>,
    brightness_key: Vec<f32>,
    distance: Vec<f32>, // zero means no usable distance
    motion_bound: Vec<f32>,
    ids: Vec<StarId>,
    names: Vec<u32>, // zero means absent, otherwise index + 1
    name_table: Vec<NameId>,
    designations: Vec<Option<Designation>>,
    spectral_types: Vec<[u8; 2]>,
    colors: Vec<f32>,
    flags: Vec<u8>,            // bit 0: singular fallback, bit 1: known color
    precise_indices: Vec<u32>, // zero means compact, otherwise index + 1
    precise_motions: Vec<StellarMotion>,
}

fn pack(v: Vector3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}
fn expand(v: [f32; 3]) -> Vector3 {
    Vector3 {
        x: v[0] as f64,
        y: v[1] as f64,
        z: v[2] as f64,
    }
}
fn norm(v: Vector3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

/// Bound the angular effect of quantization at every epoch in the interval, including closest approach.
fn bound_quantization(original: StellarMotion, compact: StellarMotion) -> f64 {
    let (start, end) = computational_years();
    let error = norm(original.u0 - compact.u0) + start.abs().max(end.abs()) * norm(original.w - compact.w);
    let radius = original.closest_approach(start, end).1;
    if error >= radius {
        std::f64::consts::PI
    } else {
        (error / radius).asin()
    }
}

impl StarStorage {
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn precise_count(&self) -> usize {
        self.precise_motions.len()
    }
    pub fn id(&self, i: usize) -> StarId {
        self.ids[i]
    }
    pub fn brightness_key(&self, i: usize) -> f64 {
        self.brightness_key[i] as f64
    }
    pub fn motion_bound(&self, i: usize) -> f64 {
        self.motion_bound[i] as f64
    }
    pub fn magnitude(&self, i: usize) -> f64 {
        self.magnitude[i] as f64
    }
    pub fn stored_direction(&self, i: usize) -> Vector3 {
        expand(std::array::from_fn(|axis| self.u0[axis][i]))
    }
    pub fn motion(&self, i: usize) -> StellarMotion {
        let precise = self.precise_indices[i];
        if precise != 0 {
            return self.precise_motions[precise as usize - 1];
        }
        StellarMotion {
            u0: expand(std::array::from_fn(|axis| self.u0[axis][i])),
            w: expand(std::array::from_fn(|axis| self.w[axis][i])),
            distance_pc: (self.distance[i] > 0.0).then_some(self.distance[i] as f64),
        }
    }
    /// Materialize metadata only for selected objects; no full array of expanded stars is kept.
    pub fn get(&self, i: usize) -> Star {
        Star {
            id: self.ids[i],
            name: (self.names[i] != 0).then(|| self.name_table[self.names[i] as usize - 1]),
            designation: self.designations[i],
            motion: self.motion(i),
            magnitude: self.magnitude(i),
            brightness_key: self.brightness_key(i),
            motion_bound: self.motion_bound(i),
            singular_fallback: self.flags[i] & 1 != 0,
            spectral_type: self.spectral_types[i],
            color_index: (self.flags[i] & 2 != 0).then_some(self.colors[i]),
            has_data: true,
        }
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Star> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }

    pub(crate) fn push(&mut self, star: Star) {
        // decide singular handling on quantized inputs, then certify compact direction accuracy
        let original = star.motion;
        let mut compact = StellarMotion {
            u0: expand(pack(original.u0)),
            w: expand(pack(original.w)),
            distance_pc: original.distance_pc.map(|d| d as f32 as f64),
        };
        let singular = compact.remove_singular_distance();
        compact.w = expand(pack(compact.w));
        let distance_valid = compact.distance_pc.is_none_or(|d| d.is_finite() && d > 0.0);
        let precise = !distance_valid || (!singular && bound_quantization(original, compact) > MAX_DIRECTION_ERROR);
        let mut motion = if precise { original } else { compact };
        let singular = if precise {
            motion.remove_singular_distance()
        } else {
            singular
        };
        let precise_index = if precise {
            self.precise_motions.push(motion);
            u32::try_from(self.precise_motions.len()).expect("precision table fits in u32")
        } else {
            0
        };

        // derive conservative keys and bounds from exactly the values observation will use
        for (axis, value) in pack(compact.u0).into_iter().enumerate() {
            self.u0[axis].push(value);
        }
        for (axis, value) in pack(if precise { Vector3::default() } else { compact.w })
            .into_iter()
            .enumerate()
        {
            self.w[axis].push(value);
        }
        self.magnitude.push(star.magnitude as f32);
        self.brightness_key.push(
            (motion.brightest_magnitude(star.magnitude) as f32)
                .next_down()
                .max(f32::MIN),
        );
        self.motion_bound
            .push(((motion.motion_bound() + QUANTIZATION_MARGIN) as f32).next_up());
        self.distance.push(if precise {
            0.0
        } else {
            motion.distance_pc.unwrap_or(0.0) as f32
        });
        self.precise_indices.push(precise_index);
        self.ids.push(star.id);
        let name = star.name.map_or(0, |name| {
            self.name_table.push(name);
            u32::try_from(self.name_table.len()).expect("name table fits in u32")
        });
        self.names.push(name);
        self.designations.push(star.designation);
        self.spectral_types.push(star.spectral_type);
        self.colors.push(star.color_index.unwrap_or(0.0));
        self.flags
            .push(u8::from(star.singular_fallback || singular) | (u8::from(star.color_index.is_some()) << 1));
    }

    /// Reorder in place using a permutation, without an expanded or second compact catalog.
    pub(crate) fn reorder(&mut self, order: &[usize]) {
        let mut destination = vec![0; self.len()];
        for (new, &old) in order.iter().enumerate() {
            destination[old] = new;
        }
        for i in 0..self.len() {
            while destination[i] != i {
                let j = destination[i];
                for a in &mut self.u0 {
                    a.swap(i, j);
                }
                for a in &mut self.w {
                    a.swap(i, j);
                }
                self.magnitude.swap(i, j);
                self.brightness_key.swap(i, j);
                self.distance.swap(i, j);
                self.motion_bound.swap(i, j);
                self.ids.swap(i, j);
                self.names.swap(i, j);
                self.designations.swap(i, j);
                self.spectral_types.swap(i, j);
                self.colors.swap(i, j);
                self.flags.swap(i, j);
                self.precise_indices.swap(i, j);
                destination.swap(i, j);
            }
        }
    }
    pub(crate) fn reserve(&mut self, capacity: usize) {
        for a in &mut self.u0 {
            a.reserve(capacity);
        }
        for a in &mut self.w {
            a.reserve(capacity);
        }
        self.magnitude.reserve(capacity);
        self.brightness_key.reserve(capacity);
        self.distance.reserve(capacity);
        self.motion_bound.reserve(capacity);
        self.ids.reserve(capacity);
        self.names.reserve(capacity);
        self.designations.reserve(capacity);
        self.spectral_types.reserve(capacity);
        self.colors.reserve(capacity);
        self.flags.reserve(capacity);
        self.precise_indices.reserve(capacity);
    }
    pub(crate) fn shrink_to_fit(&mut self) {
        for a in &mut self.u0 {
            a.shrink_to_fit();
        }
        for a in &mut self.w {
            a.shrink_to_fit();
        }
        self.magnitude.shrink_to_fit();
        self.brightness_key.shrink_to_fit();
        self.distance.shrink_to_fit();
        self.motion_bound.shrink_to_fit();
        self.ids.shrink_to_fit();
        self.names.shrink_to_fit();
        self.designations.shrink_to_fit();
        self.spectral_types.shrink_to_fit();
        self.colors.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.precise_indices.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::Equatorial;
    use proptest::prelude::*;
    fn star(motion: StellarMotion, magnitude: f64) -> Star {
        Star {
            id: StarId(1),
            name: None,
            designation: None,
            motion,
            magnitude,
            brightness_key: 0.0,
            motion_bound: 0.0,
            singular_fallback: false,
            spectral_type: *b"G2",
            color_index: None,
            has_data: true,
        }
    }
    fn separation(a: Vector3, b: Vector3) -> f64 {
        norm(a.cross(b)).atan2(a.dot(b))
    }

    #[test]
    fn magnitude_edges_are_conservative_after_quantization() {
        for magnitude in [5_f32.next_down(), 5.0, 5_f32.next_up()] {
            let motion = StellarMotion::from_sky_motion(
                Equatorial {
                    right_ascension: 0.7,
                    declination: 0.4,
                },
                0.0,
                0.0,
            );
            let mut storage = StarStorage::default();
            storage.push(star(motion, magnitude as f64));
            assert!(storage.brightness_key(0) <= magnitude as f64);
            assert_eq!(
                storage.motion(0).evaluate(10000.0, storage.magnitude(0)).magnitude,
                magnitude as f64
            );
        }
    }

    #[test]
    fn close_approach_needs_sparse_precision_to_meet_half_arcsecond_limit() {
        let mut worst = 0.0_f64;
        for longitude in [0.3, 0.7, 1.1, 1.9, 2.2] {
            let u0 = Equatorial {
                right_ascension: longitude,
                declination: 0.4,
            }
            .to_unit_vector();
            let tangent = Vector3 {
                x: -longitude.sin(),
                y: longitude.cos(),
                z: 0.0,
            };
            let original = StellarMotion {
                u0,
                w: u0 * -0.001 + tangent * 1.01e-6,
                distance_pc: Some(1.0),
            };
            let compact = StellarMotion {
                u0: expand(pack(u0)),
                w: expand(pack(original.w)),
                ..original
            };
            let t = original.closest_approach(-10000.0, 10000.0).0;
            worst = worst.max(separation(
                original.evaluate(t, 5.0).direction,
                compact.evaluate(t, 5.0).direction,
            ));
            let mut storage = StarStorage::default();
            storage.push(star(original, 5.0));
            assert_eq!(storage.precise_count(), 1);
            assert_eq!(storage.motion(0), original);
        }
        eprintln!("naive f32 close-approach error: {} arcsec", worst.to_degrees() * 3600.0);
        assert!(worst > MAX_DIRECTION_ERROR);
    }

    proptest! {
        #[test]
        fn stored_trajectories_meet_precision_and_conservative_bounds(
            ra in 0.0_f64..std::f64::consts::TAU, dec in -1.57_f64..1.57,
            wx in -0.001_f64..0.001, wy in -0.001_f64..0.001, wz in -0.001_f64..0.001,
            distance in prop::bool::ANY,
        ) {
            let mut original = StellarMotion { u0: Equatorial { right_ascension: ra, declination: dec }.to_unit_vector(),
                w: Vector3 { x:wx, y:wy, z:wz }, distance_pc: distance.then_some(10.0) };
            let mut storage = StarStorage::default(); storage.push(star(original,5.0));
            original.remove_singular_distance();
            let stored = storage.motion(0);
            let (start,end) = computational_years();
            for i in 0..=32 {
                let t = if i==32 { original.closest_approach(start,end).0 } else { start + (end-start)*i as f64/31.0 };
                let sample = stored.evaluate(t,5.0);
                prop_assert!(separation(original.evaluate(t,5.0).direction,sample.direction) <= MAX_DIRECTION_ERROR + 1e-12);
                prop_assert!(storage.brightness_key(0) <= sample.magnitude);
                prop_assert!(separation(storage.stored_direction(0),sample.direction) <= storage.motion_bound(0));
            }
        }
    }
}
