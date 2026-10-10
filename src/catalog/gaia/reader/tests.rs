use super::*;
fn header() -> String { format!("# %ECSV 1.0\n# delimiter: ','\n{}\n", FIELDS.join(",")) }
fn row(id: u64) -> String { format!("{id},45,0,2016,10,-5,2,20,null,12,13,11\n") }

#[test]
fn ecsv_comments_quotes_reordered_columns_and_missing_values() {
    let text = format!("{}{}{}", header(), row(1), row(2).replace(",45,", ",,"));
    let mut stats = GaiaReadStats::default();
    let mut rows = Vec::new();
    read_sources(text.as_bytes(), 0, 0, &mut stats, |row| { rows.push(row); Ok(()) }).unwrap();
    assert_eq!((stats.rows, stats.missing_required, rows.len()), (2, 1, 1));
    assert_eq!(rows[0].ra, 45_f64.to_radians());
    assert_eq!(rows[0].radial_velocity, None);
    let columns = FIELDS.iter().rev().copied().collect::<Vec<_>>().join(",");
    let values = row(3).trim().split(',').rev().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join(",");
    read_sources(format!("{columns}\n{values}\n").as_bytes(), 0, 0, &mut stats, |row| { assert_eq!(row.id.0, 2); assert_eq!(row.source_id, 3); Ok(()) }).unwrap();
}

#[test]
fn malformed_numbers_schema_epoch_ids_and_order_fail() {
    let cases = [row(1).replace(",45,", ",360,"), row(1).replace(",2016,", ",2000,"),
        row(1).replace(",10,", ",NaN,"), row(1).replace(",12,", ",60,"), row(1).replace(",12,", ",-11,"),
        row(1).replace("1,45", "18446744073709551616,45"), format!("{}{}", row(1), row(1)), row(1_u64 << 43)];
    for case in cases { assert!(read_sources(format!("{}{case}", header()).as_bytes(), 0, 0, &mut GaiaReadStats::default(), |_| Ok(())).is_err(), "{case}"); }
    for hdr in [header().replace("source_id", "other"), header().replace("ra,dec", "ra,ra")] {
        assert!(read_sources(hdr.as_bytes(), 0, 0, &mut GaiaReadStats::default(), |_| Ok(())).is_err());
    }
}

#[test]
fn internal_id_overflow_fails_instead_of_wrapping() {
    let mut stats = GaiaReadStats { rows: u64::from(u32::MAX) + 1, missing_required: 0 };
    assert!(read_sources(format!("{}{}", header(), row(1)).as_bytes(), 0, 0, &mut stats, |_| Ok(())).is_err());
}

#[test]
fn truncated_gzip_is_not_a_successful_short_catalog() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
    gzip.write_all(format!("{}{}", header(), row(1)).as_bytes()).unwrap();
    let bytes = gzip.finish().unwrap();
    assert!(read_sources(MultiGzDecoder::new(&bytes[..bytes.len() - 4]), 0, 0, &mut GaiaReadStats::default(), |_| Ok(())).is_err());
}
