//! Pack the published VSOP87E series files of `data/vsop87/` into one table, `vsop87e.bin` in `OUT_DIR`, which
//! `formulas/constants.rs` embeds (`VSOP87E_TERMS`). The text files stay the readable, verifiable source
//! (`data/vsop87/provenance.md`); this packed form is only a build artefact.
//!
//! Each term of a series contributes `a·cos(b + c·t)`, `t` in thousands of Julian years from J2000; a coordinate is
//! the sum over the powers `k` of `t^k` times the series of power `k`. The amplitudes, phases and frequencies are
//! parsed as doubles exactly as printed, in file order, so the program sums the same numbers the solution publishes.
//!
//! Layout, all little-endian: the magic `VSOP87E1`; then per body, in `BodyId` order (Sun, Mercury, Venus, Earth,
//! Mars, Jupiter, Saturn, Uranus, Neptune), 18 u32 term counts (coordinate-major X, Y, Z, then powers 0 to 5)
//! followed by that body's terms as f64 triples (a, b, c) in the same order.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const BODIES: [&str; 9] = ["sun", "mer", "ven", "ear", "mar", "jup", "sat", "ura", "nep"];     // file extensions, in BodyId order
const POWERS: usize = 6;

/// A body's series by (coordinate, power): the count its header announces and its (a, b, c) rows.
type Series = BTreeMap<(usize, usize), (usize, Vec<[f64; 3]>)>;

fn main() {
    let mut packed = b"VSOP87E1".to_vec();
    for extension in BODIES {
        let path = format!("data/vsop87/VSOP87E.{extension}");
        println!("cargo:rerun-if-changed={path}");
        let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        pack_body(&path, &text, &mut packed);
    }
    let output = Path::new(&std::env::var("OUT_DIR").unwrap()).join("vsop87e.bin");
    fs::write(&output, packed).unwrap_or_else(|error| panic!("{}: {error}", output.display()));
}

/// Append one body's 18 counts and its terms, checking every series against the count its header announces.
fn pack_body(path: &str, text: &str, packed: &mut Vec<u8>) {
    // the series by (coordinate, power)
    let mut series = Series::new();
    let mut current = None;
    for line in text.lines() {
        if line.contains("VSOP87 VERSION") {
            let key = parse_header(path, line);
            series.insert(key, (parse_announced_count(path, line), Vec::new()));
            current = Some(key);
            continue;
        }
        if line.trim().is_empty() { continue; }
        let key = current.unwrap_or_else(|| panic!("{path}: a term before the first header"));
        series.get_mut(&key).unwrap().1.push(parse_term(path, line));
    }
    for (key, (announced, rows)) in &series {
        assert_eq!(rows.len(), *announced, "{path}: series {key:?} holds {} terms, its header announced {announced}", rows.len());
    }

    // the counts, coordinate-major, then the terms in the same order
    let mut terms = Vec::new();
    for coordinate in 1..=3 {
        for power in 0..POWERS {
            let rows = series.get(&(coordinate, power)).map(|(_, rows)| rows.as_slice()).unwrap_or(&[]);
            packed.extend_from_slice(&(rows.len() as u32).to_le_bytes());
            terms.extend_from_slice(rows);
        }
    }
    for [a, b, c] in terms {
        packed.extend_from_slice(&a.to_le_bytes());
        packed.extend_from_slice(&b.to_le_bytes());
        packed.extend_from_slice(&c.to_le_bytes());
    }
}

/// `(coordinate, power)` from a header such as ` VSOP87 VERSION E5    EARTH     VARIABLE 1 (XYZ)       *T**0   1199 TERMS ...`.
fn parse_header(path: &str, line: &str) -> (usize, usize) {
    let coordinate = field_after(line, "VARIABLE ").and_then(|text| text.split_whitespace().next()?.parse().ok());
    let power = field_after(line, "*T**").and_then(|text| text[..1].parse().ok());
    match (coordinate, power) {
        (Some(coordinate @ 1..=3), Some(power)) if power < POWERS => (coordinate, power),
        _ => panic!("{path}: unexpected header {line:?}"),
    }
}

/// The term count a header announces.
fn parse_announced_count(path: &str, line: &str) -> usize {
    let before_terms = line.split(" TERMS").next().unwrap_or_default();
    before_terms.split_whitespace().last().and_then(|text| text.parse().ok()).unwrap_or_else(|| panic!("{path}: no term count in {line:?}"))
}

/// The amplitude, phase and frequency of a term record, from the fixed columns 80–97, 98–111 and 112–131.
fn parse_term(path: &str, line: &str) -> [f64; 3] {
    assert!(line.len() >= 131, "{path}: short record {line:?}");
    let columns = [&line[79..97], &line[97..111], &line[111..131]];
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert!(fields.len() >= 3 && columns.iter().zip(&fields[fields.len() - 3..]).all(|(column, field)| column.trim() == *field), "{path}: unexpected record layout {line:?}");
    let mut term = [0.0; 3];
    for (value, column) in term.iter_mut().zip(columns) {
        *value = column.trim().parse().unwrap_or_else(|_| panic!("{path}: not a number in {line:?}"));
    }
    term
}

fn field_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|index| &line[index + marker.len()..])
}
