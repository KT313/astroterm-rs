//! Loader for the AT-HYG star catalog (Augmented Tycho-HYG, <https://codeberg.org/astronexus/athyg>): a CSV file of
//! about 2.5 million stars from Tycho-2, Gaia DR3 and HYG, plain or gzip-compressed.
//!
//! Columns are found by their header names, so their order doesn't matter. Used here: `ra` (hours) and `dec` (degrees),
//! epoch and equinox J2000; `mag` (V); `pmra` (multiplied by cos dec) and `pmdec` in milliarcseconds per year; `ci`
//! (B-V); `spect`; `proper`; and `bayer`, `flam`, `con`, `hr`, `hip`, `tyc`, `gaia` for designations. Missing values
//! are empty fields. `dist` and `x0/y0/z0` are parsecs; `vx/vy/vz` and `rv` are km/s. Validated space-motion
//! inputs feed the stellar family; precise RA/Dec define the initial direction rather than rounded x0/y0/z0.

use std::f64::consts::PI;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::str::FromStr;

use csv::{ByteRecord, StringRecord};
use flate2::read::MultiGzDecoder;

use super::space_motion::prepare_space_motion;
use super::{
    Catalog, CatalogError, CatalogStar, Designation, StarId, StarNames, load_constellation_figures,
    load_embedded_catalog,
};
use crate::astro::{Equatorial, Vector3};

/// First bytes of a gzip file.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

const MILLIARCSECONDS_TO_RADIANS: f64 = PI / 180.0 / 3600.0 / 1000.0;

/// Stars brighter than this are the Sun, which AT-HYG lists too but the sky draws as a planet.
const SUN_MAGNITUDE_LIMIT: f64 = -20.0;

/// Positions of the columns used, if present. Only position and magnitude are required.
struct Columns {
    ra: usize,
    dec: usize,
    mag: usize,
    dist: Option<usize>,
    x0: Option<usize>,
    y0: Option<usize>,
    z0: Option<usize>,
    vx: Option<usize>,
    vy: Option<usize>,
    vz: Option<usize>,
    rv: Option<usize>,
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
    load_athyg_catalog_with_times(path, &mut crate::timing::StepTimes::default())
}

pub(crate) fn load_athyg_catalog_with_times(
    path: &Path,
    times: &mut crate::timing::StepTimes,
) -> Result<Catalog, CatalogError> {
    let describe_error = |error: io::Error| CatalogError::Io(format!("cannot read {}: {error}", path.display()));

    // decompress if the file starts like a gzip file
    let mut file = BufReader::new(File::open(path).map_err(describe_error)?);
    let is_gzip = file.fill_buf().map_err(describe_error)?.starts_with(&GZIP_MAGIC);
    let reader: Box<dyn Read> = if is_gzip {
        Box::new(MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };

    parse_athyg_with_times(reader, times).map_err(|error| match error {
        CatalogError::Io(message) => CatalogError::Io(format!("cannot read {}: {message}", path.display())),
        other => other,
    })
}

/// Parse the stars of AT-HYG CSV data.
#[cfg(test)]
fn parse_athyg(reader: impl Read) -> Result<Catalog, CatalogError> {
    parse_athyg_with_times(reader, &mut crate::timing::StepTimes::default())
}

fn parse_athyg_with_times(reader: impl Read, times: &mut crate::timing::StepTimes) -> Result<Catalog, CatalogError> {
    let (stars, names, row, missing, sun) =
        times.measure("CSV validation and parsing", || -> Result<_, CatalogError> {
            let mut csv = csv::Reader::from_reader(reader);
            let columns = find_columns(csv.headers().map_err(|error| CatalogError::Io(error.to_string()))?)?;

            let mut stars = Vec::new();
            let mut names = StarNames::default();
            let mut row = 0_u64;
            let mut missing = 0;
            let mut sun = 0;
            let mut record = ByteRecord::new();
            while csv
                .read_byte_record(&mut record)
                .map_err(|error| CatalogError::Io(error.to_string()))?
            {
                let line = record.position().map_or(0, |position| position.line());
                if let Some(star) = parse_star(&record, &columns, line, StarId::try_from_index(row)?, &mut names)? {
                    Catalog::check_star_count(stars.len() as u64 + 1)?;
                    stars.push(star);
                } else if [columns.ra, columns.dec, columns.mag]
                    .into_iter()
                    .any(|i| read_text(&record, Some(i)).is_none())
                {
                    missing += 1;
                } else {
                    sun += 1;
                }
                row += 1;
            }
            Ok((stars, names, row, missing, sun))
        })?;
    times.describe("CSV validation and parsing", || format!("input rows={row}; removed missing RA/Dec/magnitude={missing}; then removed Sun (mag < -20)={sun}; output stars={}; malformed/non-finite inputs abort loading", stars.len()));
    let mut catalog = times.measure("Catalog assembly", || -> Result<_, CatalogError> {
        Ok(Catalog::new(stars, names, load_constellation_figures()?))
    })?;
    times.describe("Catalog assembly", || {
        format!(
            "stars={}; HR representatives={}; figures={}; duplicate HR companions retained",
            catalog.stars.len(),
            catalog.hr_representatives.len(),
            catalog.constellations.len()
        )
    });
    times.measure("BSC magnitude overrides", || apply_bsc5_magnitudes(&mut catalog))?;
    times.describe("BSC magnitude overrides", || {
        format!(
            "output stars={}; membership unchanged; overrides apply to matching HR representatives",
            catalog.stars.len()
        )
    });
    Ok(catalog)
}

/// Locate the columns by their header names. Older versions name the proper motion columns `pm_ra` and `pm_dec`.
fn find_columns(headers: &StringRecord) -> Result<Columns, CatalogError> {
    let find = |name: &str| headers.iter().position(|header| header == name);
    let require = |name: &'static str| find(name).ok_or(CatalogError::MissingColumn(name));
    Ok(Columns {
        ra: require("ra")?,
        dec: require("dec")?,
        mag: require("mag")?,
        dist: find("dist"),
        x0: find("x0"),
        y0: find("y0"),
        z0: find("z0"),
        vx: find("vx"),
        vy: find("vy"),
        vz: find("vz"),
        rv: find("rv"),
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
fn parse_star(
    record: &ByteRecord,
    columns: &Columns,
    line: u64,
    id: StarId,
    names: &mut StarNames,
) -> Result<Option<CatalogStar>, CatalogError> {
    let text = |column: Option<usize>| read_text(record, column);
    let number = |column: Option<usize>, name: &'static str| parse_finite_number(record, column, name, line);

    // validate every numeric input even in skipped rows and incomplete triples
    let ra = number(Some(columns.ra), "ra")?;
    let dec = number(Some(columns.dec), "dec")?;
    let magnitude = number(Some(columns.mag), "mag")?;
    let color_index = parse_finite_f32(record, columns.ci, "ci", line)?;
    let pm_ra = number(columns.pmra, "pmra")?.unwrap_or(0.0) * MILLIARCSECONDS_TO_RADIANS;
    let pm_dec = number(columns.pmdec, "pmdec")?.unwrap_or(0.0) * MILLIARCSECONDS_TO_RADIANS;
    let distance = number(columns.dist, "dist")?;
    let position = read_triple([
        number(columns.x0, "x0")?,
        number(columns.y0, "y0")?,
        number(columns.z0, "z0")?,
    ]);
    let velocity = read_triple([
        number(columns.vx, "vx")?,
        number(columns.vy, "vy")?,
        number(columns.vz, "vz")?,
    ]);
    let radial_velocity = number(columns.rv, "rv")?;
    let hr = parse_number::<u32>(record, columns.hr, "hr", line)?;
    let hip = parse_number::<u32>(record, columns.hip, "hip", line)?;
    let gaia = parse_number::<u64>(record, columns.gaia, "gaia", line)?;
    parse_number::<u16>(record, columns.flam, "flam", line)?; // validate even when Bayer is preferred
    if ra.is_some_and(|value| !(0.0..24.0).contains(&value)) {
        return Err(CatalogError::MalformedDatasetRow { line, column: "ra" });
    }
    if dec.is_some_and(|value| !(-90.0..=90.0).contains(&value)) {
        return Err(CatalogError::MalformedDatasetRow { line, column: "dec" });
    }

    // position and brightness, without which a star can't be drawn
    let (Some(ra_hours), Some(dec_degrees), Some(magnitude)) = (ra, dec, magnitude) else {
        return Ok(None);
    };
    if magnitude < SUN_MAGNITUDE_LIMIT {
        return Ok(None);
    }
    crate::catalog::validate_magnitude(magnitude).map_err(|error| CatalogError::Io(format!("Dataset line {line}, star {}: {error}", id.0)))?;
    let declination = dec_degrees.to_radians();

    // proper motion; pmra is the motion on the sky, so divide by cos(dec) to get the change of the right ascension
    let cos_dec = declination.cos();
    let ra_motion = if cos_dec.abs() < 1e-9 { 0.0 } else { pm_ra / cos_dec };

    // color, name and designation
    let spectral_type = match text(columns.spect).map(str::as_bytes) {
        Some([class, subclass, ..]) => [*class, *subclass],
        Some([class]) => [*class, b' '],
        _ => *b"  ",
    };
    let name = text(columns.proper).map(|name| names.insert(name)).transpose().map_err(|e| CatalogError::Io(e.to_string()))?;
    let designation = select_designation(record, columns, hr, hip, gaia);

    let space_motion = prepare_space_motion(
        distance,
        position,
        velocity,
        Equatorial {
            right_ascension: (ra_hours * 15.0).to_radians(),
            declination,
        },
        Equatorial {
            right_ascension: pm_ra,
            declination: pm_dec,
        },
        radial_velocity,
    );

    // reject finite source numbers whose derived motion cannot be represented over the computational interval
    if !ra_motion.is_finite() {
        return Err(CatalogError::MalformedDatasetRow { line, column: "pmra" });
    }
    if let Some(space) = space_motion {
        let (start, end) = crate::astro::models::stars::computational_years();
        let span = start.abs().max(end.abs());
        if ![space.velocity.x, space.velocity.y, space.velocity.z]
            .into_iter()
            .map(|v| v / space.distance_pc * span)
            .fold(0.0_f64, f64::hypot)
            .is_finite()
        {
            return Err(CatalogError::MalformedDatasetRow { line, column: "dist" });
        }
    }

    Ok(Some(CatalogStar {
        id,
        space_motion,
        hr,
        name,
        designation,
        right_ascension: (ra_hours * 15.0).to_radians(),
        declination,
        ra_motion,
        ra_motion_cos_dec: pm_ra,
        dec_motion: pm_dec,
        magnitude,
        spectral_type,
        color_index,
        has_data: true,
    }))
}

/// The most readable designation a star has: Bayer, Flamsteed, then HR, HIP, Tycho-2 and Gaia numbers.
fn select_designation(
    record: &ByteRecord,
    columns: &Columns,
    hr: Option<u32>,
    hip: Option<u32>,
    gaia: Option<u64>,
) -> Option<Designation> {
    let text = |column: Option<usize>| read_text(record, column);
    let constellation = text(columns.con).unwrap_or("");

    let bayer = text(columns.bayer).and_then(|letter| Designation::parse_bayer(letter, constellation));
    let flamsteed = || text(columns.flam).and_then(|number| Designation::parse_flamsteed(number, constellation));
    let catalog_number = || -> Option<Designation> {
        if let Some(hr) = hr {
            return Some(Designation::Hr(hr));
        }
        if let Some(hip) = hip {
            return Some(Designation::Hip(hip));
        }
        if let Some(tycho) = text(columns.tyc).and_then(Designation::parse_tycho) {
            return Some(tycho);
        }
        gaia.map(Designation::Gaia)
    };
    match bayer.or_else(flamsteed) {
        Some(designation) => Some(designation),
        None => catalog_number(),
    }
}

/// Override only the previously selected representative; BSC5 placeholders contribute nothing.
fn apply_bsc5_magnitudes(catalog: &mut Catalog) -> Result<(), CatalogError> {
    let bsc5 = load_embedded_catalog()?;
    for star in &mut catalog.stars {
        let Some(hr) = star.hr else {
            continue;
        };
        if catalog.hr_representatives.get(&hr) != Some(&star.id) {
            continue;
        }
        if let Some(reference) = hr
            .checked_sub(1)
            .and_then(|index| bsc5.stars.get(index as usize))
            .filter(|s| s.has_data)
        {
            star.magnitude = reference.magnitude;
        }
    }
    Ok(())
}

fn read_triple(values: [Option<f64>; 3]) -> Option<Vector3> {
    Some(Vector3 {
        x: values[0]?,
        y: values[1]?,
        z: values[2]?,
    })
}

fn parse_finite_number(
    record: &ByteRecord,
    column: Option<usize>,
    name: &'static str,
    line: u64,
) -> Result<Option<f64>, CatalogError> {
    let value = parse_number::<f64>(record, column, name, line)?;
    if value.is_some_and(|number| !number.is_finite()) {
        return Err(CatalogError::MalformedDatasetRow { line, column: name });
    }
    Ok(value)
}

fn parse_finite_f32(
    record: &ByteRecord,
    column: Option<usize>,
    name: &'static str,
    line: u64,
) -> Result<Option<f32>, CatalogError> {
    let value = parse_finite_number(record, column, name, line)?.map(|number| number as f32);
    if value.is_some_and(|number| !number.is_finite()) {
        return Err(CatalogError::MalformedDatasetRow { line, column: name });
    }
    Ok(value)
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
    let Some(bytes) = column
        .and_then(|column| record.get(column))
        .filter(|bytes| !bytes.is_empty())
    else {
        return Ok(None);
    };
    let malformed = || CatalogError::MalformedDatasetRow { line, column: name };
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    text.parse().map(Some).map_err(|_| malformed())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    #[test]
    fn all_used_numeric_fields_reject_non_finite_values_with_line_numbers() {
        for column in [
            "ra", "dec", "mag", "pmra", "pmdec", "ci", "dist", "x0", "y0", "z0", "vx", "vy", "vz", "rv", "hr", "hip",
            "gaia", "flam",
        ] {
            for bad in ["NaN", "inf", "-inf", "broken"] {
                let csv = if ["ra", "dec", "mag"].contains(&column) {
                    let fields = ["ra", "dec", "mag"].map(|name| if name == column { bad } else { "1" });
                    format!("ra,dec,mag\n{}\n", fields.join(","))
                } else {
                    format!("ra,dec,mag,{column}\n1,2,3,{bad}\n")
                };
                assert_eq!(
                    parse_athyg(csv.as_bytes()).unwrap_err(),
                    CatalogError::MalformedDatasetRow { line: 2, column }
                );
            }
        }
        for column in ["mag", "ci"] {
            let csv = if column == "mag" {
                "ra,dec,mag\n1,2,1e100\n".to_owned()
            } else {
                "ra,dec,mag,ci\n1,2,3,1e100\n".to_owned()
            };
            assert!(parse_athyg(csv.as_bytes()).is_err());
        }
    }

    #[test]
    fn coordinate_ranges_and_missing_required_values() {
        for (ra, dec, column) in [
            ("24", "0", "ra"),
            ("-0.1", "0", "ra"),
            ("0", "90.1", "dec"),
            ("0", "-90.1", "dec"),
        ] {
            let csv = format!("ra,dec,mag\n{ra},{dec},1\n");
            assert_eq!(
                parse_athyg(csv.as_bytes()).unwrap_err(),
                CatalogError::MalformedDatasetRow { line: 2, column }
            );
        }
        let catalog = parse_athyg(b"ra,dec,mag\n0,-90,1\n23.99,90,1\n,0,1\n0,,1\n0,0,\n".as_slice()).unwrap();
        assert_eq!(catalog.stars.len(), 2);
        assert!(
            catalog
                .stars
                .iter()
                .all(|star| star.ra_motion == 0.0 && star.dec_motion == 0.0)
        );
        assert!(parse_athyg(b"ra,dec,mag,vx\n,,1,NaN\n".as_slice()).is_err());
    }

    fn parse_motion_row(row: &str) -> CatalogStar {
        let csv = format!("ra,dec,mag,dist,x0,y0,z0,vx,vy,vz,pmra,pmdec,rv\n{row}\n");
        parse_athyg(csv.as_bytes()).unwrap().stars.remove(0)
    }

    #[test]
    fn unreliable_distances_use_only_angular_motion() {
        for distance in ["", "0", "-1", "100000", "100001", "10.2"] {
            let star = parse_motion_row(&format!("0,0,1,{distance},10,0,0,1,2,3,4,5,6"));
            assert!(star.space_motion.is_none(), "distance {distance}");
            assert!(star.ra_motion > 0.0 && star.dec_motion > 0.0);
        }
        assert!(parse_motion_row("0,0,1,10,10.09,0,0,,,,,,").space_motion.is_some());
    }

    #[test]
    fn incomplete_triples_use_distance_direction_and_motion_fallbacks() {
        let star = parse_motion_row("0,0,1,10,99,,99,999,,999,1000,2000,30");
        let motion = star.space_motion.unwrap();
        assert_eq!(
            motion.position,
            Vector3 {
                x: 10.0,
                y: 0.0,
                z: 0.0
            }
        );
        let km_s = 365.25 * 86400.0 / 3.085677581491367e13;
        assert!((motion.velocity.x - 30.0 * km_s).abs() < 1e-15);
        assert!((motion.velocity.y - 10_000.0 * MILLIARCSECONDS_TO_RADIANS).abs() < 1e-15);
        assert!((motion.velocity.z - 20_000.0 * MILLIARCSECONDS_TO_RADIANS).abs() < 1e-15);
        let no_rv = parse_motion_row("0,0,1,10,,,,,,,1000,,").space_motion.unwrap();
        assert_eq!(no_rv.velocity.x, 0.0);
        assert_eq!(no_rv.velocity.z, 0.0);
        let supplied = parse_motion_row("0,0,1,10,10,0,0,1,2,3,999,999,999")
            .space_motion
            .unwrap();
        assert!((supplied.velocity.z - 3.0 * km_s).abs() < 1e-15);
    }

    #[test]
    fn hr_representatives_are_fixed_before_bsc5_overrides() {
        // Real AT-HYG duplicates can be separate components sharing an HR; keep every component.
        let mut catalog = parse_athyg(b"ra,dec,mag,hr,proper\n0,0,1,2491,First\n0,0,-3,2491,Bright\n0,0,-3,2491,Tie\n0,0,5,92,Placeholder\n0,0,,7001,Skipped\n0,0,4,99999,Unknown\n".as_slice()).unwrap();
        assert_eq!(catalog.stars.len(), 5);
        assert_eq!(catalog.hr_representatives[&2491], StarId(1));
        assert_eq!(
            catalog.stars.iter().map(|s| s.magnitude).collect::<Vec<_>>(),
            [1.0, -1.46, -3.0, 5.0, 4.0]
        );
        assert_eq!(catalog.stars[4].id, StarId(5));
        catalog.constellations = vec![super::super::ConstellationFigure {
            abbreviation: "Test",
            segments: vec![[2491, 99999]],
        }];
        let sky = crate::sky::create_sky_from_catalog(&catalog).unwrap();
        drop(catalog);
        let bright = sky.star_views().find(|star| star.id() == StarId(1)).unwrap();
        assert_eq!(sky.star_name(&bright), Some("Bright"));
        assert_eq!(sky.star_view(0).id(), StarId(2)); // override made the original representative dimmer than its companion
        assert_eq!(sky.constellations().len(), 1);
        let [a, b] = sky.constellations()[0].segments[0];
        assert_eq!((sky.star_view(a).id(), sky.star_view(b).id()), (StarId(1), StarId(5)));
    }

    #[test]
    fn distance_unknown_polar_star_preserves_tangential_ra_motion() {
        let source = parse_motion_row("12,90,1,,,,,,,,1000,,");
        let star = crate::sky::prepare_star(&source);
        assert!(star.motion.distance_pc.is_none());
        assert!((star.motion.w.y + 1000.0 * MILLIARCSECONDS_TO_RADIANS).abs() < 1e-15);
        assert!(star.motion.evaluate(10.0, star.magnitude).direction.y < 0.0);
    }

    #[test]
    fn finite_but_unrepresentable_scaled_velocity_is_rejected() {
        let csv = b"ra,dec,mag,dist,rv\n1,2,3,1e-300,1e308\n";
        assert_eq!(
            parse_athyg(csv.as_slice()).unwrap_err(),
            CatalogError::MalformedDatasetRow {
                line: 2,
                column: "dist"
            }
        );
    }

    #[test]
    fn barnard_vectors_keep_precise_catalog_direction_and_brighten_towards_approach() {
        // AT-HYG v4 row 1794252, HIP 87937. Cartesian positions are rounded to 0.0001 pc; RA/Dec retain precision.
        let star = parse_motion_row(
            "17.96347159,4.69327996,9.776,1.8282,-0.0174,-1.822,0.1496,-5.82,117.51,80.47,-801.55,10362.39,-110.468",
        );
        let source = star.space_motion.unwrap();
        assert!((source.velocity.y - 117.51 * (365.25 * 86400.0 / 3.085677581491367e13)).abs() < 1e-15);
        let prepared = crate::sky::prepare_star(&star);
        let expected = Equatorial {
            right_ascension: star.right_ascension,
            declination: star.declination,
        }
        .to_unit_vector();
        assert_eq!(prepared.motion.u0, expected);
        assert_eq!(prepared.motion.distance_pc, Some(1.8282));
        let (start, end) = crate::astro::models::stars::computational_years();
        let (closest, ratio) = prepared.motion.closest_approach(start, end);
        assert!(closest > 0.0 && closest < end && ratio < 1.0);
        assert!(prepared.motion.evaluate(closest, prepared.magnitude).magnitude_value() < prepared.magnitude - 0.5);
    }

    /// A few rows in the AT-HYG v4 layout (shortened to the columns used plus a few others).
    const SAMPLE: &str = "\
id,tyc,gaia,hip,hr,bayer,flam,con,proper,ra,dec,dist,mag,ci,pmra,pmdec,spect
1,,,,,,,,Sol,0.0,0.0,4.85e-06,-26.74,,0.0,0.0,G2 V
2,,,69673,5340,Alp,16,Boo,Arcturus,14.26103,19.18241,11.26,-0.05,1.239,-1093.39,-2000.06,K1.5III
3,4628-237-1,,,,,61,Cyg,,21.11,38.75,3.5,5.2,1.069,,,K5V
4,1-381-1,2738327528519591936,,,,,Psc,,5.974e-05,1.08900761,184.7,9.139,0.502,-0.36,-5.05,
5,,,,,,,,,1.0,2.0,,,,,,
";

    fn parse_sample() -> Catalog {
        parse_athyg(SAMPLE.as_bytes()).expect("sample parses")
    }

    #[test]
    fn rows_without_magnitude_and_the_sun_are_skipped() {
        assert_eq!(parse_sample().stars.len(), 3);
    }

    #[test]
    fn values_are_converted_to_radians_per_year() {
        let catalog = parse_sample();
        let arcturus = &catalog.stars[0];
        assert_eq!(
            (catalog.names.get(arcturus.name), arcturus.hr),
            (Some("Arcturus"), Some(5340))
        );
        assert!((arcturus.right_ascension - (14.26103 * 15.0_f64).to_radians()).abs() < 1e-12);
        assert!((arcturus.declination - 19.18241_f64.to_radians()).abs() < 1e-12);
        let mas = MILLIARCSECONDS_TO_RADIANS;
        let expected_ra_motion = -1093.39 * mas / 19.18241_f64.to_radians().cos();
        assert!((arcturus.ra_motion - expected_ra_motion).abs() < 1e-15);
        assert!((arcturus.dec_motion - -2000.06 * mas).abs() < 1e-15);
        assert_eq!((arcturus.magnitude, arcturus.color_index), (-0.04, Some(1.239)));
        assert_eq!(&arcturus.spectral_type, b"K1");
    }

    #[test]
    fn designations_prefer_bayer_then_flamsteed_then_catalog_numbers() {
        let stars = parse_sample().stars;
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
