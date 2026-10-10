//! Stream one verified compressed ECSV file, retaining just one CSV record and one converted input row.
use super::{GaiaSource, GaiaReadStats};
use crate::progress::{DatasetProgress, Stage, Unit, Outcome};
use crate::catalog::{datasets::GaiaFile, StarId, validate_magnitude};
use csv::{ByteRecord, ReaderBuilder};
use flate2::read::MultiGzDecoder;
use std::{collections::HashSet, fs::File, io::{self, BufReader, Read}};

const FIELDS: [&str; 12] = ["source_id", "ra", "dec", "ref_epoch", "pmra", "pmdec", "parallax", "parallax_over_error", "radial_velocity", "phot_g_mean_mag", "phot_bp_mean_mag", "phot_rp_mean_mag"];

pub(crate) fn read_gaia_file_with_progress(file: &GaiaFile, stats: &mut GaiaReadStats, progress: &mut dyn DatasetProgress, consume: impl FnMut(GaiaSource) -> io::Result<()>) -> io::Result<()> {
    progress.begin_stage(Stage::Parse, &file.entry.name, Unit::Rows, None);
    read_sources_with_progress(MultiGzDecoder::new(BufReader::new(File::open(&file.path)?)), file.entry.first_cell, file.entry.last_cell, stats, progress, consume)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", file.path.display())))
}

#[cfg(test)]
fn read_sources(reader: impl Read, first_cell: u32, last_cell: u32, stats: &mut GaiaReadStats, consume: impl FnMut(GaiaSource) -> io::Result<()>) -> io::Result<()> {
    read_sources_with_progress(reader, first_cell, last_cell, stats, &mut crate::progress::NoProgress(io::sink()), consume)
}

fn read_sources_with_progress(reader: impl Read, first_cell: u32, last_cell: u32, stats: &mut GaiaReadStats, progress: &mut dyn DatasetProgress, mut consume: impl FnMut(GaiaSource) -> io::Result<()>) -> io::Result<()> {
    let start = stats.rows;
    let mut csv = ReaderBuilder::new().comment(Some(b'#')).from_reader(reader);
    let headers = csv.headers().map_err(io::Error::other)?;
    let unique: HashSet<_> = headers.iter().collect();
    if unique.len() != headers.len() { return Err(invalid("duplicate Gaia header")); }
    let mut columns = [0; FIELDS.len()];
    for (index, name) in FIELDS.iter().enumerate() {
        columns[index] = headers.iter().position(|field| field == *name).ok_or_else(|| invalid(format!("missing Gaia column {name}")))?;
    }
    let mut previous = None;
    let mut record = ByteRecord::new();
    while csv.read_byte_record(&mut record).map_err(io::Error::other)? {
        let row = record.position().map_or(0, |position| position.line());
        let result = (|| {
            let id = StarId::try_from_index(stats.rows).map_err(io::Error::other)?;
            stats.rows = stats.rows.checked_add(1).ok_or_else(|| invalid("Gaia row count overflow"))?;
            let source_id = text(&record, columns[0])?.ok_or_else(|| invalid("missing source_id"))?.parse::<u64>().map_err(|_| invalid("invalid source_id"))?;
            let cell = source_id >> 43;
            if cell < u64::from(first_cell) || cell > u64::from(last_cell) { return Err(invalid(format!("source_id {source_id} outside file sky range"))); }
            if previous.is_some_and(|last| source_id <= last) { return Err(invalid(format!("source_id {source_id} duplicated or out of order"))); }
            previous = Some(source_id);
            let mut values = [None; 11];
            for i in 1..FIELDS.len() {
                values[i - 1] = text(&record, columns[i])?.map(|text| {
                    text.parse::<f64>().ok().filter(|value| value.is_finite()).ok_or_else(|| invalid(format!("invalid {} for Gaia DR3 {source_id}", FIELDS[i])))
                }).transpose()?;
            }
            let [ra, dec, epoch, pmra, pmdec, parallax, parallax_signal, radial_velocity, magnitude, bp, rp] = values;
            if ra.is_some_and(|v| !(0.0..360.0).contains(&v)) || dec.is_some_and(|v| !(-90.0..=90.0).contains(&v)) { return Err(invalid("RA/Dec outside valid range")); }
            let Some(epoch) = epoch else { return Err(invalid("missing ref_epoch")); };
            if epoch != 2016.0 { return Err(invalid("expected Gaia DR3 ref_epoch 2016.0")); }
            let (Some(ra), Some(dec), Some(magnitude)) = (ra, dec, magnitude) else { stats.missing_required += 1; return Ok(()); };
            validate_magnitude(magnitude).map_err(io::Error::other)?;
            consume(GaiaSource { id, source_id, ra: ra.to_radians(), dec: dec.to_radians(), epoch, pmra, pmdec,
                parallax, parallax_signal, radial_velocity, magnitude, bp, rp })
                .map_err(|error| io::Error::new(error.kind(), format!("Gaia DR3 {source_id}: {error}")))
        })();
        result.map_err(|error| io::Error::new(error.kind(), format!("row {row}: {error}")))?;
        if (stats.rows - start).is_multiple_of(crate::constants::STARTUP_PROGRESS_ROWS) {
            progress.set_position(stats.rows - start); progress.set_retained(stats.rows - stats.missing_required);
        }
    }
    progress.set_position(stats.rows - start); progress.set_retained(stats.rows - stats.missing_required); progress.finish_stage(Outcome::Complete);
    Ok(())
}
fn text(row: &ByteRecord, column: usize) -> io::Result<Option<&str>> {
    let bytes = row.get(column).ok_or_else(|| invalid("missing Gaia field"))?;
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("invalid UTF-8 Gaia field"))?.trim();
    Ok((!text.is_empty() && text != "null").then_some(text)) // ESA ECSV uses literal null for missing measurements
}
fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }

#[cfg(test)]
mod tests;
