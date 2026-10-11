//! Selective MAST input: pinned discovery, projected Parquet blocks and immutable local objects.
mod storage;
mod transport;
mod parquet;
mod pipeline;
mod downloading;

pub(crate) use storage::{BlobStore, digest, encode, decode};
pub(crate) use parquet::{MastRow, read_block};
pub(crate) use pipeline::{MastBlock, MastSelection, resolve_mast};

pub(crate) const COLUMNS: [&str; 13] = ["source_id", "designation", "ra", "dec", "ref_epoch", "pmra", "pmdec", "parallax", "parallax_over_error", "radial_velocity", "phot_g_mean_mag", "phot_bp_mean_mag", "phot_rp_mean_mag"];
