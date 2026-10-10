//! Gaia DR3 ECSV parsing only. Conversion into our sky model belongs to the sky layer.
mod reader;
pub(crate) use reader::read_gaia_file;

#[derive(Clone, Copy, Debug)]
pub(crate) struct GaiaSource {
    pub id: super::StarId,
    pub source_id: u64,
    pub ra: f64,
    pub dec: f64,
    pub epoch: f64,
    pub pmra: Option<f64>,
    pub pmdec: Option<f64>,
    pub parallax: Option<f64>,
    pub parallax_signal: Option<f64>,
    pub radial_velocity: Option<f64>,
    pub magnitude: f64,
    pub bp: Option<f64>,
    pub rp: Option<f64>,
}

#[derive(Default, Debug)]
pub(crate) struct GaiaReadStats {
    pub rows: u64,
    pub missing_required: u64,
}
