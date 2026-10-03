//! Truncated ELP-2000/82, Meeus second edition chapter 47. All arithmetic is f64.
//! Output is geometric mean ecliptic/equinox of date, radians and kilometres, TT.
#[path = "terms.rs"]
mod terms;
use crate::astro::J2000;

pub fn compute_lunar_ecliptic(tt: f64) -> (f64, f64, f64) {
    // fundamental arguments, reduced before trigonometry
    let t = (tt - J2000) / 36525.0;
    let radians = |degrees: f64| degrees.rem_euclid(360.0).to_radians();
    let l =
        radians(218.3164477 + 481267.88123421 * t - 0.0015786 * t * t + t.powi(3) / 538841.0 - t.powi(4) / 65194000.0);
    let d =
        radians(297.8501921 + 445267.1114034 * t - 0.0018819 * t * t + t.powi(3) / 545868.0 - t.powi(4) / 113065000.0);
    let m = radians(357.5291092 + 35999.0502909 * t - 0.0001536 * t * t + t.powi(3) / 24490000.0);
    let mp =
        radians(134.9633964 + 477198.8675055 * t + 0.0087414 * t * t + t.powi(3) / 69699.0 - t.powi(4) / 14712000.0);
    let f =
        radians(93.2720950 + 483202.0175233 * t - 0.0036539 * t * t - t.powi(3) / 3526000.0 + t.powi(4) / 863310000.0);
    let e = 1.0 - 0.002516 * t - 0.0000074 * t * t;
    let argument = |row: &[f64]| row[0] * d + row[1] * m + row[2] * mp + row[3] * f;

    // periodic longitude, distance and latitude
    let (mut longitude, mut distance, mut latitude) = (0.0, 0.0, 0.0);
    for row in terms::LONGITUDE_DISTANCE {
        let (s, c) = argument(&row).sin_cos();
        let factor = e.powi(row[1].abs() as i32);
        longitude += row[4] * factor * s;
        distance += row[5] * factor * c;
    }
    for row in terms::LATITUDE {
        latitude += row[4] * e.powi(row[1].abs() as i32) * argument(&row).sin();
    }

    // additive planetary/figure-of-Earth terms, in the tables' microdegree units
    let a1 = radians(119.75 + 131.849 * t);
    let a2 = radians(53.09 + 479264.290 * t);
    let a3 = radians(313.45 + 481266.484 * t);
    longitude += 3958.0 * a1.sin() + 1962.0 * (l - f).sin() + 318.0 * a2.sin();
    latitude +=
        -2235.0 * l.sin() + 382.0 * a3.sin() + 175.0 * (a1 - f).sin() + 175.0 * (a1 + f).sin() + 127.0 * (l - mp).sin()
            - 115.0 * (l + mp).sin();
    (
        l + (longitude / 1e6).to_radians(),
        (latitude / 1e6).to_radians(),
        385000.56 + distance / 1000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meeus_47a_geometric_example() {
        let (l, b, r) = compute_lunar_ecliptic(2448724.5);
        assert!((l.to_degrees() - 133.162655).abs() < 0.000001);
        assert!((b.to_degrees() + 3.229126).abs() < 0.000001);
        assert!((r - 368409.7).abs() < 0.1);
    }
}
