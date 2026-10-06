//! Immutable structure-of-arrays storage. Arithmetic is f64 after expanding the compact inputs; only a sparse
//! exception table retains trajectories whose certified quantization error would exceed half an arcsecond.
mod views;
use crate::model::Star;
use crate::astro::{
    Vector3,
    models::stars::{StellarMotion, computational_years},
};
use crate::catalog::cache::{
    CatalogArray, MappedCatalog,
    encoding::{decode_designation, encode_designation},
    invalid,
};
use crate::catalog::{NameId, StarId};
use std::{io, sync::Arc};

/// The grid uses the effective stored trajectory, so this covers cell-direction rounding and f64 bound
/// arithmetic, not the original catalog's quantization error. Model error is certified separately below.
pub const QUANTIZATION_MARGIN: f64 = 0.1 * std::f64::consts::PI / (180.0 * 3600.0);
const MAX_DIRECTION_ERROR: f64 = 0.5 * std::f64::consts::PI / (180.0 * 3600.0);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StarStorage {
    u0: [CatalogArray<f32>; 3],
    w: [CatalogArray<f32>; 3],
    magnitude: CatalogArray<f32>,
    brightness_key: CatalogArray<f32>,
    distance: CatalogArray<f32>, // zero means no usable distance
    motion_bound: CatalogArray<f32>,
    ids: CatalogArray<u64>,
    names: CatalogArray<u32>, // zero means absent, otherwise index + 1
    name_table: CatalogArray<[u64; 2]>,
    designations: CatalogArray<[u8; 16]>,
    spectral_types: CatalogArray<[u8; 2]>,
    colors: CatalogArray<f32>,
    flags: CatalogArray<u8>,            // bit 0: singular fallback, bit 1: known color
    precise_indices: CatalogArray<u32>, // zero means compact, otherwise index + 1
    precise_motions: CatalogArray<[f64; 7]>,
}

fn encode_motion(m: StellarMotion) -> [f64; 7] {
    [
        m.u0.x,
        m.u0.y,
        m.u0.z,
        m.w.x,
        m.w.y,
        m.w.z,
        m.distance_pc.unwrap_or(0.0),
    ]
}
fn decode_motion(m: [f64; 7]) -> StellarMotion {
    StellarMotion {
        u0: Vector3 {
            x: m[0],
            y: m[1],
            z: m[2],
        },
        w: Vector3 {
            x: m[3],
            y: m[4],
            z: m[5],
        },
        distance_pc: (m[6] > 0.0).then_some(m[6]),
    }
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
    pub fn is_mapped(&self) -> bool {
        self.ids.is_mapped()
    }
    pub(crate) fn cache_sections(&self) -> Vec<&[u8]> {
        vec![
            self.u0[0].bytes(),
            self.u0[1].bytes(),
            self.u0[2].bytes(),
            self.w[0].bytes(),
            self.w[1].bytes(),
            self.w[2].bytes(),
            self.magnitude.bytes(),
            self.brightness_key.bytes(),
            self.distance.bytes(),
            self.motion_bound.bytes(),
            self.ids.bytes(),
            self.names.bytes(),
            self.name_table.bytes(),
            self.designations.bytes(),
            self.spectral_types.bytes(),
            self.colors.bytes(),
            self.flags.bytes(),
            self.precise_indices.bytes(),
            self.precise_motions.bytes(),
        ]
    }
    pub(crate) fn from_mapping(m: &Arc<MappedCatalog>) -> io::Result<Self> {
        Ok(Self {
            u0: [
                CatalogArray::from_mapping(m, 0)?,
                CatalogArray::from_mapping(m, 1)?,
                CatalogArray::from_mapping(m, 2)?,
            ],
            w: [
                CatalogArray::from_mapping(m, 3)?,
                CatalogArray::from_mapping(m, 4)?,
                CatalogArray::from_mapping(m, 5)?,
            ],
            magnitude: CatalogArray::from_mapping(m, 6)?,
            brightness_key: CatalogArray::from_mapping(m, 7)?,
            distance: CatalogArray::from_mapping(m, 8)?,
            motion_bound: CatalogArray::from_mapping(m, 9)?,
            ids: CatalogArray::from_mapping(m, 10)?,
            names: CatalogArray::from_mapping(m, 11)?,
            name_table: CatalogArray::from_mapping(m, 12)?,
            designations: CatalogArray::from_mapping(m, 13)?,
            spectral_types: CatalogArray::from_mapping(m, 14)?,
            colors: CatalogArray::from_mapping(m, 15)?,
            flags: CatalogArray::from_mapping(m, 16)?,
            precise_indices: CatalogArray::from_mapping(m, 17)?,
            precise_motions: CatalogArray::from_mapping(m, 18)?,
        })
    }
    pub(crate) fn validate(&self, names: &crate::catalog::StarNames, full: bool) -> io::Result<()> {
        let n = self.len();
        if self.u0.iter().chain(&self.w).any(|a| a.len() != n)
            || [
                self.magnitude.len(),
                self.brightness_key.len(),
                self.distance.len(),
                self.motion_bound.len(),
                self.names.len(),
                self.designations.len(),
                self.spectral_types.len(),
                self.colors.len(),
                self.flags.len(),
                self.precise_indices.len(),
            ]
            .iter()
            .any(|&len| len != n)
        {
            return Err(invalid("star-array length mismatch"));
        }
        for &range in self.name_table.iter() {
            if names.get(Some(NameId::from_range(range))).is_none() {
                return Err(invalid("invalid name range"));
            }
        }
        for m in self.precise_motions.iter() {
            let motion = decode_motion(*m);
            let (start, end) = computational_years();
            if m.iter().any(|v| !v.is_finite())
                || m[6] < 0.0
                || (norm(motion.u0) - 1.0).abs() > 1e-6
                || !norm(motion.w * start.abs().max(end.abs())).is_finite()
            {
                return Err(invalid("invalid precise trajectory"));
            }
        }
        let mut precise_seen = vec![false; self.precise_motions.len()];
        for i in 0..n {
            if self.names[i] as usize > self.name_table.len()
                || self.precise_indices[i] as usize > self.precise_motions.len()
                || self.flags[i] > 3
                || decode_designation(self.designations[i]).is_none()
            {
                return Err(invalid("invalid star metadata/index"));
            }
            if self.u0.iter().chain(&self.w).any(|a| !a[i].is_finite())
                || [
                    self.magnitude[i],
                    self.brightness_key[i],
                    self.distance[i],
                    self.motion_bound[i],
                    self.colors[i],
                ]
                .iter()
                .any(|x| !x.is_finite())
                || self.distance[i] < 0.0
                || self.motion_bound[i] < 0.0
            {
                return Err(invalid("invalid numerical star data"));
            }
            if (norm(self.stored_direction(i)) - 1.0).abs() > 1e-6 {
                return Err(invalid("non-unit stored direction"));
            }
            let precision = self.precise_indices[i];
            if precision != 0 {
                if std::mem::replace(&mut precise_seen[precision as usize - 1], true) {
                    return Err(invalid("duplicate precision index"));
                }
                if self.distance[i] != 0.0 || self.w.iter().any(|a| a[i] != 0.0) {
                    return Err(invalid("invalid precision placeholder"));
                }
            }
            if full {
                let mut motion = self.motion(i);
                if (norm(motion.u0) - 1.0).abs() > 1e-6
                    || motion.remove_singular_distance()
                    || (self.flags[i] & 1 != 0 && motion.distance_pc.is_some())
                    || self.brightness_key(i) > motion.brightest_magnitude(self.magnitude(i))
                    || self.motion_bound(i) < motion.motion_bound() + QUANTIZATION_MARGIN
                {
                    return Err(invalid("inconsistent trajectory policy/bounds"));
                }
            }
        }
        if precise_seen.iter().any(|&seen| !seen) {
            return Err(invalid("unreferenced precision entry"));
        }
        Ok(())
    }
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
        StarId(self.ids[i])
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
            return decode_motion(self.precise_motions[precise as usize - 1]);
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
            id: StarId(self.ids[i]),
            name: (self.names[i] != 0).then(|| NameId::from_range(self.name_table[self.names[i] as usize - 1])),
            designation: decode_designation(self.designations[i]).expect("validated designation"),
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
            self.precise_motions.push(encode_motion(motion));
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
        self.ids.push(star.id.0);
        let name = star.name.map_or(0, |name| {
            self.name_table.push(name.range());
            u32::try_from(self.name_table.len()).expect("name table fits in u32")
        });
        self.names.push(name);
        self.designations.push(encode_designation(star.designation));
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

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarStorage { u0, w, magnitude, brightness_key, distance, motion_bound, ids, names, name_table, designations, spectral_types, colors, flags, precise_indices, precise_motions });

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
