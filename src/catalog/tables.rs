//! Parsers for the text tables: star names (`bsc5_names.txt`) and constellation figures (`bsc5_constellations.txt`).

use super::CatalogError;

/// Stick figure of a constellation as line segments between BSC5 catalog numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstellationFigure {
    pub abbreviation: &'static str,
    pub segments: Vec<[u32; 2]>,
}

/// Parse `catalog_number,name` lines into a table indexed by `catalog_number - 1`.
pub fn parse_star_names(text: &'static str, star_count: usize) -> Result<Vec<Option<&'static str>>, CatalogError> {
    let mut names = vec![None; star_count];
    for (line_index, line) in non_empty_lines(text) {
        let malformed = || CatalogError::MalformedStarName { line: line_index + 1 };
        let (number, name) = line.split_once(',').ok_or_else(malformed)?;
        let catalog_number: usize = number.trim().parse().map_err(|_| malformed())?;
        let slot = catalog_number
            .checked_sub(1)
            .and_then(|index| names.get_mut(index))
            .ok_or_else(malformed)?;
        *slot = Some(name.trim());
    }
    Ok(names)
}

/// Parse lines of the form `CVn 1 4785 4915`: abbreviation, number of segments, then two catalog numbers per segment.
pub fn parse_constellation_figures(text: &'static str) -> Result<Vec<ConstellationFigure>, CatalogError> {
    non_empty_lines(text)
        .map(|(line_index, line)| parse_constellation_line(line, line_index + 1))
        .collect()
}

fn parse_constellation_line(line: &'static str, line_number: usize) -> Result<ConstellationFigure, CatalogError> {
    let malformed = || CatalogError::MalformedConstellation { line: line_number };
    let mut tokens = line.split_whitespace();

    // abbreviation and segment count
    let abbreviation = tokens.next().ok_or_else(malformed)?;
    let segment_count: usize = tokens
        .next()
        .and_then(|token| token.parse().ok())
        .ok_or_else(malformed)?;

    // exactly two catalog numbers per segment
    let numbers: Vec<u32> = tokens
        .map(|token| token.parse().map_err(|_| malformed()))
        .collect::<Result<_, _>>()?;
    if segment_count == 0 || numbers.len() != segment_count * 2 {
        return Err(malformed());
    }
    let segments = numbers.chunks_exact(2).map(|pair| [pair[0], pair[1]]).collect();
    Ok(ConstellationFigure { abbreviation, segments })
}

/// Lines with content, with their 0-based line index. Tolerates `\r\n` line endings.
fn non_empty_lines(text: &'static str) -> impl Iterator<Item = (usize, &'static str)> {
    text.lines()
        .map(str::trim_end)
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CONSTELLATIONS_TEXT, STAR_NAMES_TEXT};

    #[test]
    fn parses_reference_star_names() {
        let names = parse_star_names(STAR_NAMES_TEXT, 9110).expect("embedded names parse");
        assert_eq!(names[896], Some("Acamar"));
        assert_eq!(names[7000], Some("Vega"));
        assert_eq!(names[2692], Some("Wezen"));
        assert_eq!(names[5684], Some("Zubeneschamali"));
        assert_eq!(names[0], None);
    }

    #[test]
    fn rejects_malformed_star_names() {
        assert!(matches!(
            parse_star_names("12;Foo", 100),
            Err(CatalogError::MalformedStarName { line: 1 })
        ));
        assert!(matches!(
            parse_star_names("1,A\n200,Bar", 100),
            Err(CatalogError::MalformedStarName { line: 2 })
        ));
        assert!(matches!(
            parse_star_names("0,Zero", 100),
            Err(CatalogError::MalformedStarName { line: 1 })
        ));
    }

    #[test]
    fn parses_reference_constellations() {
        let figures = parse_constellation_figures(CONSTELLATIONS_TEXT).expect("embedded figures parse");
        assert_eq!(figures.len(), 88);
        assert_eq!((figures[0].abbreviation, figures[0].segments.len()), ("Aql", 8));
        assert_eq!(figures[19].abbreviation, "CVn");
        assert_eq!(figures[19].segments, [[4785, 4915]]);
    }

    #[test]
    fn rejects_malformed_constellations() {
        for (text, line) in [("Foo", 1), ("Foo 2 1 2 3", 1), ("Foo 0", 1), ("Ok 1 1 2\nBar 1 x 2", 2)] {
            let result = parse_constellation_figures(text);
            assert!(
                matches!(result, Err(CatalogError::MalformedConstellation { line: l }) if l == line),
                "{text}"
            );
        }
    }
}
