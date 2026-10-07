//! Immutable structure-of-arrays storage. Arithmetic is f64 after expanding the compact inputs; only a sparse
//! exception table retains trajectories whose certified quantization error would exceed half an arcsecond.
//! The per-star columns are declared once in `columns.rs`; `columns()` borrows all of them for one pass.
mod columns;
mod views;
pub use columns::{STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec};
use columns::section;
use crate::model::Star;
use crate::astro::{
    Vector3,
    models::stars::{StellarMotion, computational_years},
};
use crate::catalog::cache::{
    CatalogArray, PreparedCatalogBytes,
    encoding::{decode_designation, encode_designation},
    invalid,
};
use crate::catalog::{NameId, StarId};
use std::io;

/// The grid uses the effective stored trajectory, so this covers cell-direction rounding and f64 bound
/// arithmetic, not the original catalog's quantization error. Model error is certified separately below.
pub const QUANTIZATION_MARGIN: f64 = 0.1 * std::f64::consts::PI / (180.0 * 3600.0);
const MAX_DIRECTION_ERROR: f64 = 0.5 * std::f64::consts::PI / (180.0 * 3600.0);

#[derive(Clone, Debug, Default)]
pub struct StarStorage {
    rows: StarRowVec,                       // the declared columns; see columns.rs
    name_table: CatalogArray<[u64; 2]>,      // side table addressed by the `name` column
    precise_motions: CatalogArray<[f64; 7]>, // sparse side table addressed by the `precise_index` column
}
impl PartialEq for StarStorage {
    fn eq(&self, other: &Self) -> bool {
        self.columns() == other.columns()
            && self.name_table == other.name_table
            && self.precise_motions == other.precise_motions
    }
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
    /// Borrow the prepared columns without copying their contents.
    pub fn columns(&self) -> StarRowSlice<'_> { self.rows.as_slice() }
    /// Inspect original owned columns, including their allocated capacities.
    pub(crate) fn owned_columns(&self) -> &StarRowVec { &self.rows }
    fn rows_mut(&mut self) -> &mut StarRowVec { &mut self.rows }
    /// Name byte ranges addressed by the `name` column (1-based; 0 means unnamed).
    pub(crate) fn name_table(&self) -> &CatalogArray<[u64; 2]> {
        &self.name_table
    }
    /// Full-precision motions addressed by the `precise_index` column (1-based; 0 means none).
    pub(crate) fn precise_motions(&self) -> &CatalogArray<[f64; 7]> {
        &self.precise_motions
    }
    /// Column bytes in section order, followed by the two side tables.
    pub(crate) fn cache_sections(&self) -> Vec<&[u8]> {
        let c = self.columns();
        vec![
            bytemuck::cast_slice(c.u0),
            bytemuck::cast_slice(c.w),
            bytemuck::cast_slice(c.magnitude),
            bytemuck::cast_slice(c.brightness_key),
            bytemuck::cast_slice(c.distance),
            bytemuck::cast_slice(c.id),
            bytemuck::cast_slice(c.name),
            bytemuck::cast_slice(c.designation),
            bytemuck::cast_slice(c.spectral_type),
            bytemuck::cast_slice(c.color),
            bytemuck::cast_slice(c.flags),
            bytemuck::cast_slice(c.precise_index),
            self.name_table.bytes(),
            self.precise_motions.bytes(),
        ]
    }
    /// Load packed columns verbatim; do not quantize or reorder prepared data again.
    pub(crate) fn from_prepared(data: &PreparedCatalogBytes) -> io::Result<Self> {
        Ok(Self {
            rows: StarRowVec {
                u0: data.decode(section::U0)?,
                w: data.decode(section::W)?,
                magnitude: data.decode(section::MAGNITUDE)?,
                brightness_key: data.decode(section::BRIGHTNESS_KEY)?,
                distance: data.decode(section::DISTANCE)?,
                id: data.decode(section::ID)?,
                name: data.decode(section::NAME)?,
                designation: data.decode(section::DESIGNATION)?,
                spectral_type: data.decode(section::SPECTRAL_TYPE)?,
                color: data.decode(section::COLOR)?,
                flags: data.decode(section::FLAGS)?,
                precise_index: data.decode(section::PRECISE_INDEX)?,
            },
            name_table: data.decode(section::NAME_TABLE)?.into(),
            precise_motions: data.decode(section::PRECISE_MOTIONS)?.into(),
        })
    }
    pub(crate) fn validate(&self, names: &crate::catalog::StarNames, bounds: &[f32], full: bool) -> io::Result<()> {
        let c = self.columns();
        let n = c.id.len();
        if [
            c.u0.len(),
            c.w.len(),
            c.magnitude.len(),
            c.brightness_key.len(),
            c.distance.len(),
            bounds.len(),
            c.name.len(),
            c.designation.len(),
            c.spectral_type.len(),
            c.color.len(),
            c.flags.len(),
            c.precise_index.len(),
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
        for (i, &bound) in bounds.iter().enumerate() {
            if c.name[i] as usize > self.name_table.len()
                || c.precise_index[i] as usize > self.precise_motions.len()
                || c.flags[i] > 3
                || decode_designation(c.designation[i]).is_none()
            {
                return Err(invalid("invalid star metadata/index"));
            }
            if c.u0[i].iter().chain(&c.w[i]).any(|v| !v.is_finite())
                || [c.magnitude[i], c.brightness_key[i], c.distance[i], bound, c.color[i]]
                    .iter()
                    .any(|x| !x.is_finite())
                || c.distance[i] < 0.0
                || bound < 0.0
            {
                return Err(invalid("invalid numerical star data"));
            }
            if (norm(expand(c.u0[i])) - 1.0).abs() > 1e-6 {
                return Err(invalid("non-unit stored direction"));
            }
            let precision = c.precise_index[i];
            if precision != 0 {
                if std::mem::replace(&mut precise_seen[precision as usize - 1], true) {
                    return Err(invalid("duplicate precision index"));
                }
                if c.distance[i] != 0.0 || c.w[i] != [0.0; 3] {
                    return Err(invalid("invalid precision placeholder"));
                }
            }
            if full {
                let mut motion = self.motion(i);
                if (norm(motion.u0) - 1.0).abs() > 1e-6
                    || motion.remove_singular_distance()
                    || (c.flags[i] & 1 != 0 && motion.distance_pc.is_some())
                    || self.brightness_key(i) > motion.brightest_magnitude(self.magnitude(i))
                    || f64::from(bound) < motion.motion_bound() + QUANTIZATION_MARGIN
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
        self.rows.id.as_slice().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn precise_count(&self) -> usize {
        self.precise_motions.len()
    }
    pub fn id(&self, i: usize) -> StarId {
        StarId(self.rows.id.as_slice()[i])
    }
    pub fn brightness_key(&self, i: usize) -> f64 {
        self.rows.brightness_key.as_slice()[i] as f64
    }
    pub fn magnitude(&self, i: usize) -> f64 {
        self.rows.magnitude.as_slice()[i] as f64
    }
    pub fn stored_direction(&self, i: usize) -> Vector3 {
        expand(self.rows.u0.as_slice()[i])
    }
    pub fn motion(&self, i: usize) -> StellarMotion {
        let precise = self.rows.precise_index.as_slice()[i];
        if precise != 0 {
            return decode_motion(self.precise_motions[precise as usize - 1]);
        }
        let distance = self.rows.distance.as_slice()[i];
        StellarMotion {
            u0: expand(self.rows.u0.as_slice()[i]),
            w: expand(self.rows.w.as_slice()[i]),
            distance_pc: (distance > 0.0).then_some(distance as f64),
        }
    }
    /// Materialize metadata only for selected objects; no full array of expanded stars is kept.
    pub fn get(&self, i: usize) -> Star {
        self.star(&self.columns(), i)
    }
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Star> + '_ {
        let columns = self.columns();
        (0..self.len()).map(move |i| self.star(&columns, i))
    }
    fn star(&self, c: &StarRowSlice<'_>, i: usize) -> Star {
        Star {
            id: StarId(c.id[i]),
            name: (c.name[i] != 0).then(|| NameId::from_range(self.name_table[c.name[i] as usize - 1])),
            designation: decode_designation(c.designation[i]).expect("validated designation"),
            motion: self.motion(i),
            magnitude: f64::from(c.magnitude[i]),
            brightness_key: f64::from(c.brightness_key[i]),
            singular_fallback: c.flags[i] & 1 != 0,
            spectral_type: c.spectral_type[i],
            color_index: (c.flags[i] & 2 != 0).then_some(c.color[i]),
            has_data: true,
        }
    }

    pub(crate) fn push(&mut self, star: Star) -> f32 {
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
        let name = star.name.map_or(0, |name| {
            self.name_table.push(name.range());
            u32::try_from(self.name_table.len()).expect("name table fits in u32")
        });

        // derive conservative keys and bounds from exactly the values observation will use
        self.rows_mut().push(StarRow {
            u0: pack(compact.u0),
            w: pack(if precise { Vector3::default() } else { compact.w }),
            magnitude: star.magnitude as f32,
            brightness_key: (motion.brightest_magnitude(star.magnitude) as f32)
                .next_down()
                .max(f32::MIN),
            distance: if precise { 0.0 } else { motion.distance_pc.unwrap_or(0.0) as f32 },
            id: star.id.0,
            name,
            designation: encode_designation(star.designation),
            spectral_type: star.spectral_type,
            color: star.color_index.unwrap_or(0.0),
            flags: u8::from(star.singular_fallback || singular) | (u8::from(star.color_index.is_some()) << 1),
            precise_index,
        });
        ((motion.motion_bound() + QUANTIZATION_MARGIN) as f32).next_up()
    }

    /// Reorder in place using a permutation, without an expanded or second compact catalog.
    pub(crate) fn reorder(&mut self, order: &[usize], bounds: &mut [f32]) {
        let mut destination = vec![0; self.len()];
        for (new, &old) in order.iter().enumerate() {
            destination[old] = new;
        }
        let rows = self.rows_mut();
        for i in 0..destination.len() {
            while destination[i] != i {
                let j = destination[i];
                rows.u0.swap(i, j);
                rows.w.swap(i, j);
                rows.magnitude.swap(i, j);
                rows.brightness_key.swap(i, j);
                rows.distance.swap(i, j);
                bounds.swap(i, j);
                rows.id.swap(i, j);
                rows.name.swap(i, j);
                rows.designation.swap(i, j);
                rows.spectral_type.swap(i, j);
                rows.color.swap(i, j);
                rows.flags.swap(i, j);
                rows.precise_index.swap(i, j);
                destination.swap(i, j);
            }
        }
    }
    pub(crate) fn reserve(&mut self, capacity: usize) {
        self.rows_mut().reserve(capacity);
    }
    pub(crate) fn shrink_to_fit(&mut self) {
        self.rows_mut().shrink_to_fit();
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarRowVec { u0, w, magnitude, brightness_key, distance, id, name, designation, spectral_type, color, flags, precise_index });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarStorage { rows, name_table, precise_motions });

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

    #[test]
    fn columns_sections_and_owned_rows_follow_one_declaration_order() {
        let mut storage = StarStorage::default();
        for i in 0..3_u32 {
            let mut entry = star(
                StellarMotion::from_sky_motion(Equatorial { right_ascension: 0.1 * f64::from(i), declination: 0.2 }, 0.0, 0.0),
                4.0 + f64::from(i),
            );
            entry.id = StarId(u64::from(i));
            storage.push(entry);
        }
        let sections = storage.cache_sections();
        assert_eq!(sections.len(), STAR_SECTIONS);
        let c = storage.columns();
        assert_eq!(sections[section::U0].len(), c.u0.len() * 12);
        assert_eq!(sections[section::ID].len(), c.id.len() * 8);
        assert_eq!(sections[section::FLAGS].len(), c.flags.len());
        assert_eq!(c.id, &[0, 1, 2]);
        assert_eq!(storage.directions().shape(), &[3, 3]);
        for i in 0..3 {
            assert_eq!(storage.directions().row(i).to_vec(), c.u0[i]);
            assert_eq!(storage.get(i).id, storage.id(i));
        }
        let mut owned = storage.clone();
        let mut bounds = [0.1, 0.2, 0.3];
        owned.reorder(&[2, 0, 1], &mut bounds);
        assert_eq!(bounds, [0.3, 0.1, 0.2]);
        assert_eq!(owned.columns().id, &[2, 0, 1]);
        assert_eq!(owned.get(0), storage.get(2));
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
            let mut storage = StarStorage::default(); let bound = storage.push(star(original,5.0));
            original.remove_singular_distance();
            let stored = storage.motion(0);
            let (start,end) = computational_years();
            for i in 0..=32 {
                let t = if i==32 { original.closest_approach(start,end).0 } else { start + (end-start)*i as f64/31.0 };
                let sample = stored.evaluate(t,5.0);
                prop_assert!(separation(original.evaluate(t,5.0).direction,sample.direction) <= MAX_DIRECTION_ERROR + 1e-12);
                prop_assert!(storage.brightness_key(0) <= sample.magnitude);
                prop_assert!(separation(storage.stored_direction(0),sample.direction) <= f64::from(bound));
            }
        }
    }
}
