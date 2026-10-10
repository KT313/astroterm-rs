//! Convert Gaia inputs once, then use the same compact storage, sorting and grid preparation as other catalogs.
mod conversion;
use crate::catalog::{datasets::GaiaFile, read_gaia_file, GaiaReadStats, StarNames};
use crate::model::{PreparedCatalog, StarStorage};
use crate::timing::StepTimes;
use std::{collections::HashMap, io::{self, Write}};
use conversion::{convert_source, ConversionStats};

pub(super) fn prepare_gaia_catalog(files: &[GaiaFile], notices: &mut impl Write, times: &mut StepTimes) -> io::Result<PreparedCatalog> {
    let mut stars = StarStorage::default();
    let mut bounds = Vec::new();
    let mut names = StarNames::default();
    let mut parsed = GaiaReadStats::default();
    let mut converted = ConversionStats::default();
    times.measure("Gaia parsing and conversion", || {
        for file in files {
            read_gaia_file(file, &mut parsed, |source| {
                crate::catalog::Catalog::check_star_count(stars.len() as u64 + 1).map_err(io::Error::other)?;
                let mut star = convert_source(source, &mut converted)?;
                star.name = Some(names.insert(&format!("Gaia DR3 {}", source.source_id))?); // IDs are unique; no label deduplication map is needed
                stars.try_reserve(1)?;
                bounds.try_reserve(1).map_err(io::Error::other)?;
                bounds.push(stars.push(star)?);
                Ok(())
            })?;
            if !file.is_unchanged()? { return Err(io::Error::other("Gaia source changed during conversion")); }
        }
        Ok::<_, io::Error>(())
    })?;
    let summary = format!("rows={}; skipped missing RA/Dec/G={}; output sources={}; missing proper motion={}; angular-only distance fallback={}; assumed zero radial velocity={}; missing color={}; saturated color={}",
        parsed.rows, parsed.missing_required, stars.len(), converted.missing_motion, converted.angular_only,
        converted.missing_rv, converted.missing_color, converted.saturated_color);
    writeln!(notices, "Gaia DR3: {summary}")?;
    times.describe("Gaia parsing and conversion", || summary);
    stars.shrink_to_fit();
    bounds.shrink_to_fit();
    times.measure("Catalog preparation", || super::pipeline::finish_catalog(stars, bounds, names, &HashMap::new(), &[]))
}
