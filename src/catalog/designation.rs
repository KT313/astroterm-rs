//! Star designations from catalogs, used as labels for stars without a proper name: Bayer (α Vir), Flamsteed
//! (61 Cyg), and catalog numbers (HR, HIP, TYC, Gaia).

/// Greek letters in Bayer order, as abbreviated in the HYG catalogs and as symbols.
const GREEK_LETTERS: [(&str, char); 24] = [
    ("Alp", 'α'),
    ("Bet", 'β'),
    ("Gam", 'γ'),
    ("Del", 'δ'),
    ("Eps", 'ε'),
    ("Zet", 'ζ'),
    ("Eta", 'η'),
    ("The", 'θ'),
    ("Iot", 'ι'),
    ("Kap", 'κ'),
    ("Lam", 'λ'),
    ("Mu", 'μ'),
    ("Nu", 'ν'),
    ("Xi", 'ξ'),
    ("Omi", 'ο'),
    ("Pi", 'π'),
    ("Rho", 'ρ'),
    ("Sig", 'σ'),
    ("Tau", 'τ'),
    ("Ups", 'υ'),
    ("Phi", 'φ'),
    ("Chi", 'χ'),
    ("Psi", 'ψ'),
    ("Ome", 'ω'),
];

/// Superscript digits for numbered Bayer components (κ¹, κ²).
const SUPERSCRIPT_DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];

/// How a star is designated in a catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Designation {
    /// A Greek letter (index into the 24 letters), an optional component number, and a constellation.
    Bayer {
        letter: u8,
        component: u8,
        constellation: [u8; 3],
    },
    Flamsteed {
        number: u16,
        constellation: [u8; 3],
    },
    /// Harvard Revised / Yale Bright Star Catalogue number.
    Hr(u32),
    /// Hipparcos number.
    Hip(u32),
    /// Tycho-2 region, number and component.
    Tycho {
        region: u16,
        number: u16,
        component: u8,
    },
    Gaia(u64),
}

impl Designation {
    /// A Bayer designation from the HYG form of the letter (`"Alp"`, `"Kap-1"`) and a constellation abbreviation.
    /// `None` for letters that aren't Greek (a few stars use Latin letters).
    pub fn parse_bayer(letter: &str, constellation: &str) -> Option<Designation> {
        let (name, component) = match letter.split_once('-') {
            Some((name, component)) => (name, component.parse().ok()?),
            None => (letter, 0),
        };
        let letter = GREEK_LETTERS
            .iter()
            .position(|(abbreviation, _)| *abbreviation == name)? as u8;
        Some(Designation::Bayer {
            letter,
            component,
            constellation: parse_constellation(constellation)?,
        })
    }

    /// A Flamsteed designation from its number and a constellation abbreviation.
    pub fn parse_flamsteed(number: &str, constellation: &str) -> Option<Designation> {
        Some(Designation::Flamsteed {
            number: number.parse().ok()?,
            constellation: parse_constellation(constellation)?,
        })
    }

    /// A Tycho-2 designation from its `region-number-component` form, e.g. `"4628-237-1"`.
    pub fn parse_tycho(text: &str) -> Option<Designation> {
        let mut parts = text.split('-');
        let designation = Designation::Tycho {
            region: parts.next()?.parse().ok()?,
            number: parts.next()?.parse().ok()?,
            component: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(designation)
    }

    /// The designation as text, with Greek letters and superscripts if `unicode`, e.g. `α² Cen` or `Alp2 Cen`.
    pub fn format(&self, unicode: bool) -> String {
        match *self {
            Designation::Bayer {
                letter,
                component,
                constellation,
            } => {
                let (abbreviation, symbol) = GREEK_LETTERS[usize::from(letter)];
                let mut text = if unicode {
                    symbol.to_string()
                } else {
                    abbreviation.to_string()
                };
                if component > 0 {
                    let digits = component.to_string();
                    if unicode {
                        text.extend(
                            digits
                                .bytes()
                                .map(|digit| SUPERSCRIPT_DIGITS[usize::from(digit - b'0')]),
                        );
                    } else {
                        text.push_str(&digits);
                    }
                }
                format!("{text} {}", format_constellation(constellation))
            }
            Designation::Flamsteed { number, constellation } => {
                format!("{number} {}", format_constellation(constellation))
            }
            Designation::Hr(number) => format!("HR {number}"),
            Designation::Hip(number) => format!("HIP {number}"),
            Designation::Tycho {
                region,
                number,
                component,
            } => format!("TYC {region}-{number}-{component}"),
            Designation::Gaia(number) => format!("Gaia {number}"),
        }
    }
}

/// A three-letter constellation abbreviation such as `"Vir"`.
fn parse_constellation(text: &str) -> Option<[u8; 3]> {
    text.as_bytes().try_into().ok()
}

fn format_constellation(constellation: [u8; 3]) -> String {
    String::from_utf8_lossy(&constellation).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bayer_designations_use_greek_letters_and_superscripts() {
        let spica = Designation::parse_bayer("Alp", "Vir").unwrap();
        assert_eq!(
            (spica.format(true), spica.format(false)),
            ("α Vir".into(), "Alp Vir".into())
        );
        let kappa = Designation::parse_bayer("Kap-1", "Scl").unwrap();
        assert_eq!(
            (kappa.format(true), kappa.format(false)),
            ("κ¹ Scl".into(), "Kap1 Scl".into())
        );
        let omega = Designation::parse_bayer("Ome-12", "Sco").unwrap();
        assert_eq!(omega.format(true), "ω¹² Sco");
    }

    #[test]
    fn non_greek_or_malformed_bayer_letters_are_rejected() {
        assert_eq!(Designation::parse_bayer("p", "Eri"), None);
        assert_eq!(Designation::parse_bayer("Alp-x", "Vir"), None);
        assert_eq!(Designation::parse_bayer("Alp", "Virgo"), None);
    }

    #[test]
    fn catalog_numbers_are_formatted_with_their_prefix() {
        let flamsteed = Designation::parse_flamsteed("61", "Cyg").unwrap();
        assert_eq!(flamsteed.format(true), "61 Cyg");
        assert_eq!(Designation::Hr(5056).format(false), "HR 5056");
        assert_eq!(Designation::Hip(65474).format(true), "HIP 65474");
        assert_eq!(Designation::Gaia(42).format(true), "Gaia 42");
        let tycho = Designation::parse_tycho("4628-237-1").unwrap();
        assert_eq!(tycho.format(true), "TYC 4628-237-1");
    }

    #[test]
    fn malformed_tycho_numbers_are_rejected() {
        assert_eq!(Designation::parse_tycho("4628-237"), None);
        assert_eq!(Designation::parse_tycho("4628-237-1-2"), None);
        assert_eq!(Designation::parse_tycho("a-b-c"), None);
    }
}
