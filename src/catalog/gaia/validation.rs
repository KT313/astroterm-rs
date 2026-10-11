//! Shared scalar validation for CSV and Parquet Gaia inputs.
use crate::catalog::{validate_magnitude, GaiaSource, StarId};
use std::io;

fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }

pub(crate) fn validate_gaia_input(id: StarId, source_id: u64, values: [Option<f64>; 11]) -> io::Result<Option<GaiaSource>> {
    if values.iter().flatten().any(|value| !value.is_finite()) { return Err(invalid("nonfinite Gaia measurement")); }
    let [ra, dec, epoch, pmra, pmdec, parallax, parallax_signal, radial_velocity, magnitude, bp, rp] = values;
    if ra.is_some_and(|value| !(0.0..360.0).contains(&value)) || dec.is_some_and(|value| !(-90.0..=90.0).contains(&value)) {
        return Err(invalid("RA/Dec outside valid range"));
    }
    if epoch != Some(2016.0) { return Err(invalid("expected Gaia DR3 ref_epoch 2016.0")); }
    let (Some(ra), Some(dec), Some(magnitude)) = (ra, dec, magnitude) else { return Ok(None); };      // preserve a missing-row result separately from malformed data
    validate_magnitude(magnitude).map_err(io::Error::other)?;
    Ok(Some(GaiaSource { id, source_id, ra: ra.to_radians(), dec: dec.to_radians(), epoch: 2016.0,
        pmra, pmdec, parallax, parallax_signal, radial_velocity, magnitude, bp, rp }))
}
