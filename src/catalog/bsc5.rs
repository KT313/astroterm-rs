//! Parser for the binary Yale Bright Star Catalog, 5th edition (<http://tdc-www.harvard.edu/catalogs/bsc5.html>).
//!
//! The file is a 28 byte header followed by 32 byte little-endian entries, sorted by catalog number.

use super::CatalogError;

const HEADER_BYTES: usize = 28;
const ENTRY_BYTES: usize = 32;

/// One star of the catalog. Positions are J2000 in radians, proper motion in radians per year.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bsc5Entry {
    pub catalog_number: u32,
    pub right_ascension: f64,
    pub declination: f64,
    /// Two character spectral type, e.g. `b"A0"`. Blank for entries without data.
    pub spectral_type: [u8; 2],
    pub magnitude: f32,
    pub ra_motion: f64,
    pub dec_motion: f64,
}

impl Bsc5Entry {
    /// Whether the catalog has actual data for this star. A few catalog numbers are kept as empty placeholders
    /// (e.g. HR 92, objects later found not to be stars), with zero position and magnitude.
    pub fn has_data(&self) -> bool {
        !(self.right_ascension == 0.0 && self.declination == 0.0 && self.magnitude == 0.0)
    }
}

/// Parse all entries of a binary BSC5 file.
pub fn parse_bsc5(data: &[u8]) -> Result<Vec<Bsc5Entry>, CatalogError> {
    // header: STARN (the entry count) is negative when coordinates are J2000, which they are in BSC5
    let header = data
        .get(..HEADER_BYTES)
        .ok_or(CatalogError::TruncatedBsc5 { entry: None })?;
    let star_count = read_i32(header, 8).unsigned_abs() as usize;

    // fixed size entries after the header
    let entries = &data[HEADER_BYTES..];
    (0..star_count)
        .map(|index| {
            let bytes = entries.get(index * ENTRY_BYTES..(index + 1) * ENTRY_BYTES);
            bytes
                .map(parse_entry)
                .ok_or(CatalogError::TruncatedBsc5 { entry: Some(index) })
        })
        .collect()
}

fn parse_entry(bytes: &[u8]) -> Bsc5Entry {
    Bsc5Entry {
        catalog_number: read_f32(bytes, 0) as u32, // XNO is stored as a float
        right_ascension: read_f64(bytes, 4),
        declination: read_f64(bytes, 12),
        spectral_type: [bytes[20], bytes[21]],
        magnitude: f32::from(read_i16(bytes, 22)) / 100.0, // stored as magnitude * 100
        ra_motion: f64::from(read_f32(bytes, 24)),
        dec_motion: f64::from(read_f32(bytes, 28)),
    }
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("slice of 2 bytes"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice of 4 bytes"))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice of 4 bytes"))
}

fn read_f64(bytes: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("slice of 8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::BSC5_DATA;

    const EPSILON: f64 = 0.01;
    const MOTION_EPSILON: f64 = 1e-20;

    #[test]
    fn parses_every_entry_in_catalog_order() {
        let entries = parse_bsc5(BSC5_DATA).expect("embedded catalog parses");
        assert_eq!(entries.len(), 9110);
        assert!(
            entries
                .iter()
                .enumerate()
                .all(|(index, entry)| entry.catalog_number as usize == index + 1)
        );
    }

    #[test]
    fn parses_reference_entries() {
        let entries = parse_bsc5(BSC5_DATA).expect("embedded catalog parses");

        let first = entries[0];
        assert!((first.right_ascension - 0.023).abs() < EPSILON && (first.declination - 0.789).abs() < EPSILON);
        assert!((first.ra_motion + 0.00000005817764048288).abs() < MOTION_EPSILON);
        assert!((first.dec_motion + 0.00000008726646427704).abs() < MOTION_EPSILON);
        assert!((first.magnitude - 6.7).abs() < 0.01);

        let middle = entries[2024];
        assert!((middle.right_ascension - 1.53876226281558).abs() < EPSILON);
        assert!((middle.declination - 0.690704355203134).abs() < EPSILON);
        assert!((middle.ra_motion + 0.000000126051560300766).abs() < MOTION_EPSILON);
        assert!((middle.magnitude - 6.45).abs() < 0.01);

        let last = entries[9109];
        assert!((last.right_ascension - 0.022267).abs() < EPSILON && (last.declination - 1.070134).abs() < EPSILON);
        assert!((last.ra_motion - 0.0000000727220523799588).abs() < MOTION_EPSILON);
        assert!((last.magnitude - 5.8).abs() < 0.01);
    }

    #[test]
    fn identifies_placeholder_entries() {
        let entries = parse_bsc5(BSC5_DATA).expect("embedded catalog parses");
        let placeholders: Vec<u32> = entries
            .iter()
            .filter(|e| !e.has_data())
            .map(|e| e.catalog_number)
            .collect();
        assert_eq!(
            placeholders,
            [
                92, 95, 182, 1057, 1841, 2472, 2496, 3515, 3671, 6309, 6515, 7189, 7539, 8296
            ]
        );
    }

    #[test]
    fn rejects_truncated_data() {
        assert!(matches!(
            parse_bsc5(&BSC5_DATA[..10]),
            Err(CatalogError::TruncatedBsc5 { entry: None })
        ));
        let cut = &BSC5_DATA[..HEADER_BYTES + ENTRY_BYTES * 3 + 5];
        assert!(matches!(
            parse_bsc5(cut),
            Err(CatalogError::TruncatedBsc5 { entry: Some(3) })
        ));
    }
}
