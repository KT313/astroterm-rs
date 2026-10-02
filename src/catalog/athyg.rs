//! Loader for the AT-HYG star catalog (Augmented Tycho-HYG, <https://codeberg.org/astronexus/athyg>): a CSV file of
//! about 2.5 million stars from Tycho-2, Gaia DR3 and HYG, plain or gzip-compressed.
//!
//! Columns are found by their header names, so their order doesn't matter. Used here: `ra` (hours) and `dec` (degrees),
//! epoch and equinox J2000; `mag` (V); `pmra` (multiplied by cos dec) and `pmdec` in milliarcseconds per year; `ci`
//! (B-V); `spect`; `proper`; and `bayer`, `flam`, `con`, `hr`, `hip`, `tyc`, `gaia` for designations. Missing values
//! are empty fields.

use std::f64::consts::PI;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::str::FromStr;

use csv::{ByteRecord, StringRecord};
use flate2::read::MultiGzDecoder;

use super::{Catalog, CatalogError, CatalogStar, Designation, load_constellation_figures};

/// First bytes of a gzip file.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

const MILLIARCSECONDS_TO_RADIANS: f64 = PI / 180.0 / 3600.0 / 1000.0;

/// Stars brighter than this are the Sun, which AT-HYG lists too but the sky draws as a planet.
const SUN_MAGNITUDE_LIMIT: f32 = -20.0;

/// Positions of the columns used, if present. Only position and magnitude are required.
struct Columns {
    ra: usize,
    dec: usize,
    mag: usize,
    pmra: Option<usize>,
    pmdec: Option<usize>,
    ci: Option<usize>,
    spect: Option<usize>,
    proper: Option<usize>,
    bayer: Option<usize>,
    flam: Option<usize>,
    con: Option<usize>,
    hr: Option<usize>,
    hip: Option<usize>,
    tyc: Option<usize>,
    gaia: Option<usize>,
}

/// Load an AT-HYG CSV file (`.csv` or gzip-compressed `.csv.gz`), with the embedded constellation figures.
pub fn load_athyg_catalog(path: &Path) -> Result<Catalog, CatalogError> {
    let describe_error = |error: io::Error| CatalogError::Io(format!("cannot read {}: {error}", path.display()));

    // decompress if the file starts like a gzip file
    let mut file = BufReader::new(File::open(path).map_err(describe_error)?);
    let is_gzip = file.fill_buf().map_err(describe_error)?.starts_with(&GZIP_MAGIC);
    let reader: Box<dyn Read> = if is_gzip {
        Box::new(MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };

    Ok(Catalog {
        stars: parse_athyg(reader).map_err(|error| match error {
            CatalogError::Io(message) => CatalogError::Io(format!("cannot read {}: {message}", path.display())),
            other => other,
        })?,
        constellations: load_constellation_figures()?,
    })
}

/// Parse the stars of AT-HYG CSV data.
fn parse_athyg(reader: impl Read) -> Result<Vec<CatalogStar>, CatalogError> {
    let mut csv = csv::Reader::from_reader(reader);
    let columns = find_columns(csv.headers().map_err(|error| CatalogError::Io(error.to_string()))?)?;

    let mut stars = Vec::new();
    let mut record = ByteRecord::new();
    while csv
        .read_byte_record(&mut record)
        .map_err(|error| CatalogError::Io(error.to_string()))?
    {
        let line = record.position().map_or(0, |position| position.line());
        if let Some(star) = parse_star(&record, &columns, line)? {
            stars.push(star);
        }
    }
    Ok(stars)
}

/// Locate the columns by their header names. Older versions name the proper motion columns `pm_ra` and `pm_dec`.
fn find_columns(headers: &StringRecord) -> Result<Columns, CatalogError> {
    let find = |name: &str| headers.iter().position(|header| header == name);
    let require = |name: &'static str| find(name).ok_or(CatalogError::MissingColumn(name));
    Ok(Columns {
        ra: require("ra")?,
        dec: require("dec")?,
        mag: require("mag")?,
        pmra: find("pmra").or_else(|| find("pm_ra")),
        pmdec: find("pmdec").or_else(|| find("pm_dec")),
        ci: find("ci"),
        spect: find("spect"),
        proper: find("proper"),
        bayer: find("bayer"),
        flam: find("flam"),
        con: find("con"),
        hr: find("hr"),
        hip: find("hip"),
        tyc: find("tyc"),
        gaia: find("gaia"),
    })
}

/// One row as a star, converted to radians. `None` for rows without position or magnitude, and for the Sun.
fn parse_star(record: &ByteRecord, columns: &Columns, line: u64) -> Result<Option<CatalogStar>, CatalogError> {
    let text = |column: Option<usize>| read_text(record, column);
    let number = |column: Option<usize>, name: &'static str| parse_number::<f64>(record, column, name, line);

    // position and brightness, without which a star can't be drawn
    let (Some(ra_hours), Some(dec_degrees), Some(magnitude)) = (
        number(Some(columns.ra), "ra")?,
        number(Some(columns.dec), "dec")?,
        parse_number::<f32>(record, Some(columns.mag), "mag", line)?,
    ) else {
        return Ok(None);
    };
    if magnitude < SUN_MAGNITUDE_LIMIT {
        return Ok(None);
    }
    let declination = dec_degrees.to_radians();

    // proper motion; pmra is the motion on the sky, so divide by cos(dec) to get the change of the right ascension
    let pm_ra = number(columns.pmra, "pmra")?.unwrap_or(0.0) * MILLIARCSECONDS_TO_RADIANS;
    let pm_dec = number(columns.pmdec, "pmdec")?.unwrap_or(0.0) * MILLIARCSECONDS_TO_RADIANS;
    let cos_dec = declination.cos();
    let ra_motion = if cos_dec.abs() < 1e-9 { 0.0 } else { pm_ra / cos_dec };

    // color, name and designation
    let spectral_type = match text(columns.spect).map(str::as_bytes) {
        Some([class, subclass, ..]) => [*class, *subclass],
        Some([class]) => [*class, b' '],
        _ => *b"  ",
    };
    let name = text(columns.proper).map(|name| &*Box::leak(name.to_owned().into_boxed_str())); // lives for the run
    let hr = parse_number::<u32>(record, columns.hr, "hr", line)?;
    let designation = select_designation(record, columns, hr, line)?;

    Ok(Some(CatalogStar {
        hr,
        name,
        designation,
        right_ascension: (ra_hours * 15.0).to_radians(),
        declination,
        ra_motion,
        dec_motion: pm_dec,
        magnitude,
        spectral_type,
        color_index: parse_number::<f32>(record, columns.ci, "ci", line)?,
        has_data: true,
    }))
}

/// The most readable designation a star has: Bayer, Flamsteed, then HR, HIP, Tycho-2 and Gaia numbers.
fn select_designation(
    record: &ByteRecord,
    columns: &Columns,
    hr: Option<u32>,
    line: u64,
) -> Result<Option<Designation>, CatalogError> {
    let text = |column: Option<usize>| read_text(record, column);
    let constellation = text(columns.con).unwrap_or("");

    let bayer = text(columns.bayer).and_then(|letter| Designation::parse_bayer(letter, constellation));
    let flamsteed = || text(columns.flam).and_then(|number| Designation::parse_flamsteed(number, constellation));
    let catalog_number = || -> Result<Option<Designation>, CatalogError> {
        if let Some(hr) = hr {
            return Ok(Some(Designation::Hr(hr)));
        }
        if let Some(hip) = parse_number::<u32>(record, columns.hip, "hip", line)? {
            return Ok(Some(Designation::Hip(hip)));
        }
        if let Some(tycho) = text(columns.tyc).and_then(Designation::parse_tycho) {
            return Ok(Some(tycho));
        }
        Ok(parse_number::<u64>(record, columns.gaia, "gaia", line)?.map(Designation::Gaia))
    };
    match bayer.or_else(flamsteed) {
        Some(designation) => Ok(Some(designation)),
        None => catalog_number(),
    }
}

/// The text of a field, `None` if the column is absent or the field empty (or not UTF-8).
fn read_text(record: &ByteRecord, column: Option<usize>) -> Option<&str> {
    let bytes = record.get(column?)?;
    std::str::from_utf8(bytes).ok().filter(|text| !text.is_empty())
}

/// A numeric field, `None` if absent or empty; an error if present but not a number.
fn parse_number<T: FromStr>(
    record: &ByteRecord,
    column: Option<usize>,
    name: &'static str,
    line: u64,
) -> Result<Option<T>, CatalogError> {
    read_text(record, column)
        .map(|text| {
            text.parse()
                .map_err(|_| CatalogError::MalformedDatasetRow { line, column: name })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    /// A few rows in the AT-HYG v4 layout (shortened to the columns used plus a few others).
    const SAMPLE: &str = "\
id,tyc,gaia,hip,hr,bayer,flam,con,proper,ra,dec,dist,mag,ci,pmra,pmdec,spect
1,,,,,,,,Sol,0.0,0.0,4.85e-06,-26.74,,0.0,0.0,G2 V
2,,,69673,5340,Alp,16,Boo,Arcturus,14.26103,19.18241,11.26,-0.05,1.239,-1093.39,-2000.06,K1.5III
3,4628-237-1,,,,,61,Cyg,,21.11,38.75,3.5,5.2,1.069,,,K5V
4,1-381-1,2738327528519591936,,,,,Psc,,5.974e-05,1.08900761,184.7,9.139,0.502,-0.36,-5.05,
5,,,,,,,,,1.0,2.0,,,,,,
";

    fn parse_sample() -> Vec<CatalogStar> {
        parse_athyg(SAMPLE.as_bytes()).expect("sample parses")
    }

    #[test]
    fn rows_without_magnitude_and_the_sun_are_skipped() {
        assert_eq!(parse_sample().len(), 3);
    }

    #[test]
    fn values_are_converted_to_radians_per_year() {
        let arcturus = &parse_sample()[0];
        assert_eq!((arcturus.name, arcturus.hr), (Some("Arcturus"), Some(5340)));
        assert!((arcturus.right_ascension - (14.26103 * 15.0_f64).to_radians()).abs() < 1e-12);
        assert!((arcturus.declination - 19.18241_f64.to_radians()).abs() < 1e-12);
        let mas = MILLIARCSECONDS_TO_RADIANS;
        let expected_ra_motion = -1093.39 * mas / 19.18241_f64.to_radians().cos();
        assert!((arcturus.ra_motion - expected_ra_motion).abs() < 1e-15);
        assert!((arcturus.dec_motion - -2000.06 * mas).abs() < 1e-15);
        assert_eq!((arcturus.magnitude, arcturus.color_index), (-0.05, Some(1.239)));
        assert_eq!(&arcturus.spectral_type, b"K1");
    }

    #[test]
    fn designations_prefer_bayer_then_flamsteed_then_catalog_numbers() {
        let stars = parse_sample();
        let designations: Vec<String> = stars
            .iter()
            .map(|star| star.designation.unwrap().format(false))
            .collect();
        assert_eq!(designations, ["Alp Boo", "61 Cyg", "TYC 1-381-1"]);
        assert_eq!((stars[1].ra_motion, &stars[2].spectral_type), (0.0, b"  "));
    }

    #[test]
    fn gzip_files_are_read_like_plain_ones() {
        let directory = std::env::temp_dir().join(format!("astroterm-athyg-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (plain, gzipped) = (directory.join("sample.csv"), directory.join("sample.csv.gz"));
        std::fs::write(&plain, SAMPLE).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(SAMPLE.as_bytes()).unwrap();
        std::fs::write(&gzipped, encoder.finish().unwrap()).unwrap();

        let from_plain = load_athyg_catalog(&plain).unwrap();
        let from_gzip = load_athyg_catalog(&gzipped).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(from_plain.stars, from_gzip.stars);
        assert_eq!(from_gzip.constellations.len(), 88);
    }

    #[test]
    fn missing_columns_and_bad_values_are_reported() {
        let missing = parse_athyg("ra,dec\n1,2\n".as_bytes());
        assert_eq!(missing, Err(CatalogError::MissingColumn("mag")));
        let malformed = parse_athyg("ra,dec,mag\n1,2,3\n1,x,3\n".as_bytes());
        assert_eq!(
            malformed,
            Err(CatalogError::MalformedDatasetRow { line: 3, column: "dec" })
        );
        let unreadable = load_athyg_catalog(Path::new("/nonexistent/athyg.csv"));
        assert!(matches!(unreadable, Err(CatalogError::Io(message)) if message.contains("/nonexistent/athyg.csv")));
    }
}
