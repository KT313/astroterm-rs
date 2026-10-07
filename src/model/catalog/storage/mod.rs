//! Immutable structure-of-arrays storage. Arithmetic is f64 after expanding the compact inputs; only a sparse
//! exception-table skeleton reserves future support; unsupported trajectories fail preparation explicitly.
//! The per-star columns are declared once in `columns.rs`; `columns()` borrows all of them for one pass.
mod columns;
mod views;
pub(crate) use views::StellarFields;
pub use columns::{STAR_SECTIONS, StarRow, StarRowSlice, StarRowVec};
use columns::section;
use crate::model::Star;
use crate::astro::{
    Vector3,
    models::stars::{StellarMotion, computational_years},
};
use crate::catalog::cache::{
    CatalogArray, PreparedCatalogBytes,
    invalid,
};
use crate::catalog::{NameId, StarId, MagnitudeClipping, decode_magnitude, encode_magnitude, encode_brightness_bound};
use std::io;

/// The grid uses the effective stored trajectory, so this covers cell-direction rounding and f64 bound
/// arithmetic, not the original catalog's quantization error. Model error is certified separately below.
pub const QUANTIZATION_MARGIN: f64 = 0.1 * std::f64::consts::PI / (180.0 * 3600.0);
const MAX_DIRECTION_ERROR: f64 = 0.5 * std::f64::consts::PI / (180.0 * 3600.0);

#[derive(Clone, Debug, Default)]
pub struct StarStorage {
    clipping: MagnitudeClipping,            // catalog-wide counts, no per-star clipping flags
    rows: StarRowVec,                       // the declared columns; see columns.rs
    precise_motions: CatalogArray<[f64; 7]>, // reserved precise-motion payload; must remain empty until sparse exceptions are supported
}
impl PartialEq for StarStorage {
    fn eq(&self, other: &Self) -> bool {
        self.clipping == other.clipping && self.columns() == other.columns()
            && self.precise_motions == other.precise_motions
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
    /// Reserved full-precision side table; accepted catalogs always keep it empty.
    pub(crate) fn precise_motions(&self) -> &CatalogArray<[f64; 7]> {
        &self.precise_motions
    }
    pub(crate) fn validate_exception_storage(&self) -> io::Result<()> {
        if !self.precise_motions.is_empty() {
            return Err(crate::model::unsupported_star_data("prepared catalog contains unsupported/orphaned precise-motion records"));
        }
        Ok(())
    }
    /// Column bytes in section order, followed by the precise-motion side table.
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
            bytemuck::cast_slice(c.display_color),
            self.precise_motions.bytes(),
        ]
    }
    /// Load packed columns verbatim; do not quantize or reorder prepared data again.
    pub(crate) fn from_prepared(data: &PreparedCatalogBytes, clipping: MagnitudeClipping) -> io::Result<Self> {
        Ok(Self {
            clipping,
            rows: StarRowVec {
                u0: data.decode(section::U0)?,
                w: data.decode(section::W)?,
                magnitude: data.decode(section::MAGNITUDE)?,
                brightness_key: data.decode(section::BRIGHTNESS_KEY)?,
                distance: data.decode(section::DISTANCE)?,
                id: data.decode(section::ID)?,
                name: data.decode(section::NAME)?,
                display_color: data.decode(section::DISPLAY_COLOR)?,
            },
            precise_motions: data.decode(section::PRECISE_MOTIONS)?.into(),
        })
    }
    pub(crate) fn validate(&self, names: &crate::catalog::StarNames, bounds: &[f32], full: bool) -> io::Result<()> {
        let c = self.columns();
        let n = c.id.len();
        crate::catalog::Catalog::check_star_count(n as u64).map_err(io::Error::other)?;
        names.validate()?;
        self.validate_exception_storage()?;
        if [
            c.u0.len(),
            c.w.len(),
            c.magnitude.len(),
            c.brightness_key.len(),
            c.distance.len(),
            bounds.len(),
            c.name.len(),
            c.display_color.len(),
        ]
        .iter()
        .any(|&len| len != n)
        {
            return Err(invalid("star-array length mismatch"));
        }
        let mut clipping = MagnitudeClipping::default();
        for (i, &bound) in bounds.iter().enumerate() {
            if !names.contains(c.name[i])
                || crate::model::StarColor::from_index(c.display_color[i]).is_none()
            {
                return Err(invalid("invalid star metadata/index"));
            }
            if c.u0[i].iter().chain(&c.w[i]).any(|v| !v.is_finite())
                || [c.distance[i], bound]
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
            let mut motion = self.motion(i);
            if motion.remove_singular_distance() {
                return Err(crate::model::unsupported_star_data(&format!("Star {} requires tangential-motion fallback", self.id(i).0)));
            }
            let (expected, counts) = encode_brightness_bound(motion.brightest_magnitude(self.magnitude(i)));
            if c.brightness_key[i] != expected { return Err(invalid("inconsistent encoded brightness bound")); }
            clipping.lower += counts.lower;
            clipping.upper += counts.upper;
            if full && ((norm(motion.u0) - 1.0).abs() > 1e-6
                || f64::from(bound) < motion.motion_bound() + QUANTIZATION_MARGIN) {
                return Err(invalid("inconsistent trajectory policy/bounds"));
            }
        }
        if self.clipping != clipping { return Err(invalid("inconsistent brightness clipping counts")); }
        Ok(())
    }
    pub fn magnitude_clipping(&self) -> MagnitudeClipping { self.clipping }

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
        decode_magnitude(self.rows.brightness_key.as_slice()[i])
    }
    pub fn magnitude(&self, i: usize) -> f64 {
        decode_magnitude(self.rows.magnitude.as_slice()[i])
    }
    pub fn stored_direction(&self, i: usize) -> Vector3 {
        expand(self.rows.u0.as_slice()[i])
    }
    pub fn motion(&self, i: usize) -> StellarMotion {
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
            name: NameId::from_entry(c.name[i]),
            motion: self.motion(i),
            magnitude: decode_magnitude(c.magnitude[i]),
            brightness_key: decode_magnitude(c.brightness_key[i]),
            display_color: crate::model::StarColor::from_index(c.display_color[i]).expect("validated color index"),
            has_data: true,
        }
    }

    pub(crate) fn push(&mut self, star: Star) -> io::Result<f32> {
        let magnitude = encode_magnitude(star.magnitude).map_err(|error| invalid(format!("Star {}: {error}", star.id.0)))?;
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
        if singular {
            return Err(crate::model::unsupported_star_data(&format!("Star {} requires tangential-motion fallback", star.id.0)));
        }
        if precise {
            let reason = if !distance_valid { "a distance not representable as a positive finite f32" } else { "a higher-precision trajectory (compact angular error exceeds 0.5 arcseconds)" };
            return Err(crate::model::unsupported_star_data(&format!("Star {} requires {reason}", star.id.0)));
        }
        let name = star.name.map_or(0, NameId::entry);

        // derive conservative keys and bounds from exactly the values observation will use
        let (brightness_key, clipping) = encode_brightness_bound(motion.brightest_magnitude(decode_magnitude(magnitude)));
        self.clipping.lower += clipping.lower;
        self.clipping.upper += clipping.upper;
        self.rows_mut().push(StarRow {
            u0: pack(compact.u0),
            w: pack(compact.w),
            magnitude,
            brightness_key,
            distance: motion.distance_pc.unwrap_or(0.0) as f32,
            id: star.id.0,
            name,
            display_color: star.display_color.index(),
        });
        Ok(((motion.motion_bound() + QUANTIZATION_MARGIN) as f32).next_up())
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
                rows.display_color.swap(i, j);
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
crate::cache::report_fields!(StarRowVec { u0, w, magnitude, brightness_key, distance, id, name, display_color });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(StarStorage { clipping, rows, precise_motions });

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::Equatorial;
    use proptest::prelude::*;
    fn star(motion: StellarMotion, magnitude: f64) -> Star {
        Star {
            id: StarId(1),
            name: None,
            motion,
            magnitude,
            brightness_key: 0.0,
            display_color: crate::model::StarColor::Yellow,
            has_data: true,
        }
    }
    fn separation(a: Vector3, b: Vector3) -> f64 {
        norm(a.cross(b)).atan2(a.dot(b))
    }

    #[test]
    fn unsupported_fallback_and_unrepresentable_distance_leave_storage_empty() {
        let mut storage = StarStorage::default();
        let original = StellarMotion { u0: Vector3 { x: 1.0, y: 0.0, z: 0.0 },
            w: Vector3 { x: -0.001, y: 0.0, z: 0.0 }, distance_pc: Some(1.0) };
        let error = storage.push(star(original, 4.0)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("Star 1 requires tangential-motion fallback"));
        assert!(original.evaluate(1000.0, 4.0).used_singular_fallback); // numerical fallback remains available
        for distance in [1e-50, 1e40] {
            let motion = StellarMotion { w: Vector3::default(), distance_pc: Some(distance), ..original };
            let error = storage.push(star(motion, 4.0)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("distance not representable"));
        }
        assert!(storage.is_empty());
        assert!(storage.precise_motions.is_empty());
    }

    #[test]
    fn orphaned_precise_payload_is_rejected() {
        let mut storage = StarStorage::default();
        storage.precise_motions.push([0.0; 7]);
        assert_eq!(storage.validate_exception_storage().unwrap_err().kind(), io::ErrorKind::Unsupported);
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
            storage.push(star(motion, magnitude as f64)).unwrap();
            assert!(storage.brightness_key(0) <= storage.magnitude(0));
            assert_eq!(
                storage.motion(0).evaluate(10000.0, storage.magnitude(0)).magnitude,
                5.0
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
            let error = storage.push(star(original, 5.0)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("higher-precision trajectory"));
            assert!(error.to_string().contains("Star 1"));
            assert!(storage.is_empty());
            assert_eq!(storage.precise_count(), 0);
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
            entry.id = StarId(i);
            storage.push(entry).unwrap();
        }
        let sections = storage.cache_sections();
        assert_eq!(sections.len(), STAR_SECTIONS);
        let c = storage.columns();
        assert_eq!(sections[section::U0].len(), c.u0.len() * 12);
        assert_eq!(sections[section::ID].len(), c.id.len() * 4);
        assert_eq!(sections[section::DISPLAY_COLOR].len(), c.display_color.len());
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
            let mut storage = StarStorage::default();
            let result = storage.push(star(original,5.0));
            let mut compact = StellarMotion { u0: expand(pack(original.u0)), w: expand(pack(original.w)),
                distance_pc: original.distance_pc.map(|d| d as f32 as f64) };
            let fallback = compact.remove_singular_distance();
            compact.w = expand(pack(compact.w));
            if let Err(error) = &result {
                prop_assert_eq!(error.kind(), io::ErrorKind::Unsupported);
                prop_assert!(fallback || bound_quantization(original, compact) > MAX_DIRECTION_ERROR);
                prop_assert!(storage.is_empty());
                return Ok(());
            }
            let bound = result.unwrap();
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
