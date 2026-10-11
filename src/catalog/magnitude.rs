//! Two-byte catalog magnitudes. The motion model evaluates a current magnitude as f64 and stores it as a code
//! again (`magnitude_code`); only few-star callers decode.
use std::io;

pub const MIN_MAGNITUDE: f64 = -10.0;
pub const MAX_MAGNITUDE: f64 = 55.535;
pub const MAGNITUDE_CLIPPING_WARNING: &str = "Stored brightness bounds clipped; early filtering may do extra work. Current brightness saturates at the same limits.";

pub fn decode_magnitude(code: u16) -> f64 { f64::from(code) / 1000.0 - 10.0 }

/// The code of a calculated (current) magnitude: nearest thousandth, clamped to the storable range. Values below
/// -10 and NaN become code 0, values above 55.535 become `u16::MAX`. Every record after the simulation holds
/// magnitudes in this form; the raster's opacity table is indexed by it directly.
pub fn magnitude_code(value: f64) -> u16 {
    ((value - MIN_MAGNITUDE) * 1000.0 + 0.5).clamp(0.0, f64::from(u16::MAX)) as u16 // NaN stays NaN through clamp and casts to 0
}

pub fn validate_magnitude(value: f64) -> io::Result<()> {
    if !value.is_finite() || !(MIN_MAGNITUDE..=MAX_MAGNITUDE).contains(&value) {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("magnitude {value} is outside the allowed range -10.000 through 55.535")));
    }
    Ok(())
}

/// Nearest thousandth; halfway cases round toward the larger code (the fainter magnitude).
/// The tie rule applies to the scaled f64 value, not to an exact decimal reinterpretation of the source text.
pub fn encode_magnitude(value: f64) -> io::Result<u16> {
    validate_magnitude(value)?;
    let code = ((value + 10.0) * 1000.0).round() as u64;
    u16::try_from(code).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "magnitude encoding overflow"))
}

/// Counts of genuinely out-of-range calculated bounds, retained across prepared-cache loads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MagnitudeClipping {
    pub lower: u64,
    pub upper: u64,
}
impl MagnitudeClipping {
    pub fn any(self) -> bool { self.lower != 0 || self.upper != 0 }
}
crate::rows::row_columns!(MagnitudeClipping { lower, upper });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(MagnitudeClipping);

/// Encode toward brighter values. Zero deliberately bypasses all early brightness pruning.
/// `brightest_magnitude` applies one next_down for safety: that one ULP below the exact lower endpoint
/// is encoding roundoff, not a genuine clipping event. This never relaxes validation of raw magnitudes.
pub fn encode_brightness_bound(value: f64) -> (u16, MagnitudeClipping) {
    assert!(!value.is_nan(), "validated trajectory must produce a numerical brightness bound");
    if value < MIN_MAGNITUDE {
        return (0, MagnitudeClipping { lower: u64::from(value < MIN_MAGNITUDE.next_down()), upper: 0 });
    }
    if value > MAX_MAGNITUDE { return (u16::MAX, MagnitudeClipping { lower: 0, upper: 1 }); }
    let mut code = ((value + 10.0) * 1000.0).floor() as u16;
    if code != 0 && decode_magnitude(code) > value { code -= 1; }
    (code, MagnitudeClipping::default())
}

pub fn passes_brightness_bound(code: u16, threshold: f64) -> bool {
    code == 0 || decode_magnitude(code) <= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculated_magnitude_codes_round_to_thousandths_and_clamp() {
        assert_eq!(magnitude_code(2.0), 12000);
        assert_eq!(magnitude_code(2.0004), 12000);
        assert_eq!(magnitude_code(2.0006), 12001);
        assert_eq!(magnitude_code(-10.0), 0);
        assert_eq!(magnitude_code(-12.0), 0);
        assert_eq!(magnitude_code(f64::NAN), 0);
        assert_eq!(magnitude_code(55.535), u16::MAX);
        assert_eq!(magnitude_code(60.0), u16::MAX);
        for value in [-9.999, 0.0, 4.1231, 20.0, 55.534] { assert_eq!(magnitude_code(value), encode_magnitude(value).unwrap()); } // same rounding as the stored catalog codes
    }

    #[test]
    fn raw_range_and_nearest_step_ties_are_explicit() {
        assert_eq!(encode_magnitude(-10.0).unwrap(), 0);
        assert_eq!(encode_magnitude(55.535).unwrap(), u16::MAX);
        for value in [MIN_MAGNITUDE.next_down(), 55.535_f64.next_up(), f64::NAN, f64::INFINITY] {
            assert!(encode_magnitude(value).is_err(), "{value}");
        }
        assert_eq!(encode_magnitude(-9.9375).unwrap(), 63); // exact scaled half: 62.5 rounds upward
        assert_eq!(encode_magnitude(-1.2344).unwrap(), 8766);
        assert_eq!(encode_magnitude(-1.2346).unwrap(), 8765);
        for code in 0..=u16::MAX {
            assert_eq!(encode_magnitude(decode_magnitude(code)).unwrap(), code);
        }
    }

    #[test]
    fn every_bound_code_and_its_neighbors_decode_conservatively() {
        for code in 0..=u16::MAX {
            let value = decode_magnitude(code);
            for bound in [value.next_down(), value, value.next_up()] {
                let (stored, _) = encode_brightness_bound(bound);
                assert!(stored == 0 || decode_magnitude(stored) <= bound, "{code}: {bound}");
                if (MIN_MAGNITUDE..=MAX_MAGNITUDE).contains(&bound) {
                    assert!(bound - decode_magnitude(stored) < 0.0010000000001);
                }
            }
        }
    }

    #[test]
    fn clipped_and_exact_minimum_bounds_never_prune_bright_stars() {
        for bound in [-100.0, f64::NEG_INFINITY, MIN_MAGNITUDE, MIN_MAGNITUDE.next_down()] {
            let (code, counts) = encode_brightness_bound(bound);
            assert_eq!(code, 0);
            assert!(passes_brightness_bound(code, -50.0));
            assert_eq!(counts.lower, u64::from(bound < MIN_MAGNITUDE.next_down()));
        }
        let (code, counts) = encode_brightness_bound(70.0);
        assert_eq!(code, u16::MAX);
        assert_eq!(counts, MagnitudeClipping { lower: 0, upper: 1 });
        assert!(!passes_brightness_bound(code, 55.0));
        assert_eq!(encode_brightness_bound(MAX_MAGNITUDE).1, MagnitudeClipping::default());
        assert!(!passes_brightness_bound(1, -10.0));
    }
}
