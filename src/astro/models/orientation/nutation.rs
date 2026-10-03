//! IAU 2000B, translated from ERFA nut00b (77 terms, f64).
//! Copyright (C) 2013-2023 NumFOCUS Foundation; see LICENSE-ERFA.
use crate::astro::J2000;

/// Nutation in longitude and obliquity in radians at TT. Fixed planetary offsets follow IAU 2000B.
/// Its short-model linear fundamental arguments are retained, including for unvalidated extrapolation.
pub fn compute_nutation(tt: f64) -> (f64, f64) {
    let t = (tt - J2000) / 36525.0;
    let as2r = std::f64::consts::PI / (180.0 * 3600.0);
    let arguments = [
        (485868.249036, 1717915923.2178),
        (1287104.79305, 129596581.0481),
        (335779.526232, 1739527262.8478),
        (1072260.70369, 1602961601.2090),
        (450160.398036, -6962890.5431),
    ]
    .map(|(a, b)| (a + b * t).rem_euclid(1296000.0) * as2r);
    let (mut psi, mut eps) = (0.0, 0.0);
    for row in super::nutation_terms::TERMS.iter().rev() {
        let argument: f64 = row[..5].iter().zip(arguments).map(|(n, a)| n * a).sum();
        let (s, c) = argument.sin_cos();
        psi += (row[5] + row[6] * t) * s + row[7] * c;
        eps += (row[8] + row[9] * t) * c + row[10] * s;
    }
    ((psi / 1e7 - 0.000135) * as2r, (eps / 1e7 + 0.000388) * as2r)
}
