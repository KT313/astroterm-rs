//! Gaia J2016 ICRS inputs rebased to our J2000 straight-line model; G is deliberately not converted to V.
use crate::astro::{Equatorial, Vector3, models::stars::StellarMotion};
use crate::catalog::{GaiaSource, validate_magnitude};
use crate::constants::GAIA_DISTANCE_MIN_SIGNAL;
use crate::model::{Star, StarColor};
use std::io;

const MAS_TO_RADIANS: f64 = std::f64::consts::PI / (180.0 * 3_600_000.0);
const KM_S_TO_PC_YEAR: f64 = 365.25 * 86400.0 / 3.085677581491367e13;

#[derive(Default)]
pub(super) struct ConversionStats {
    pub missing_motion: u64,
    pub angular_only: u64,
    pub missing_rv: u64,
    pub missing_color: u64,
    pub saturated_color: u64,
}

pub(super) fn convert_source(source: GaiaSource, stats: &mut ConversionStats) -> io::Result<Star> {
    let direction = Equatorial { right_ascension: source.ra, declination: source.dec };
    let u = direction.to_unit_vector();
    let mut distance = source.parallax.filter(|p| *p > 0.0)
        .filter(|_| source.parallax_signal.is_some_and(|signal| signal >= GAIA_DISTANCE_MIN_SIGNAL))
        .map(|p| 1000.0 / p).filter(|d| d.is_finite() && *d > 0.0 && *d < 100_000.0);
    let w = if let (Some(pmra), Some(pmdec)) = (source.pmra, source.pmdec) {
        let tangent = StellarMotion::from_sky_motion(direction, pmra * MAS_TO_RADIANS, pmdec * MAS_TO_RADIANS).w;
        if let Some(distance) = distance {
            if source.radial_velocity.is_none() { stats.missing_rv += 1; }
            tangent + u * (source.radial_velocity.unwrap_or(0.0) * KM_S_TO_PC_YEAR / distance)
        } else { stats.angular_only += 1; tangent }
    } else {
        stats.missing_motion += 1;
        distance = None; // incomplete motion must not create invented radial-only evolution
        Vector3::default()
    };

    if ![w.x, w.y, w.z].into_iter().all(f64::is_finite) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "nonfinite derived Gaia velocity"));
    }
    // Gaia uses TCB Julian years; our existing approximate model treats them as TT Julian years.
    let q = u + w * (2000.0 - source.epoch);
    let scale = q.x.hypot(q.y).hypot(q.z);
    if !scale.is_finite() || scale <= 0.0 { return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid Gaia reference-epoch trajectory")); }
    let motion = StellarMotion { u0: q * (1.0 / scale), w: w * (1.0 / scale), distance_pc: distance.map(|d| d * scale) };
    let mut magnitude = source.magnitude; // store Gaia G in the existing V-magnitude field; no band conversion
    if distance.is_some() { magnitude += 5.0 * scale.log10(); } // match the new reference distance, not a different color band
    validate_magnitude(magnitude).map_err(io::Error::other)?;
    Ok(Star { id: source.id, name: None, motion, magnitude, brightness_key: motion.brightest_magnitude(magnitude),
        display_color: select_color(source.bp.zip(source.rp).map(|(bp, rp)| bp - rp), stats), has_data: true })
}

/// DR3 Table 5.9: observed BP-RP as a polynomial in B-V. Used only for our coarse palette boundaries.
/// https://gea.esac.esa.int/archive/documentation/GDR3/Data_processing/chap_cu5pho/cu5pho_sec_photSystem/cu5pho_ssec_photRelations.html
fn gaia_color(bv: f64) -> f64 { 0.06483 + bv * (1.575 + bv * (-0.7815 + bv * (0.5707 + bv * (-0.176 + bv * 0.01916)))) }

fn select_color(color: Option<f64>, stats: &mut ConversionStats) -> StarColor {
    let Some(color) = color else { stats.missing_color += 1; return StarColor::Default; };
    if color < gaia_color(-0.5) || color > gaia_color(3.5) { stats.saturated_color += 1; }
    if color < gaia_color(0.0) { StarColor::BlueWhite }
    else if color < gaia_color(0.8) { StarColor::Default }
    else if color < gaia_color(1.4) { StarColor::Orange }
    else { StarColor::RedOrange }
}

#[cfg(test)]
mod tests;
