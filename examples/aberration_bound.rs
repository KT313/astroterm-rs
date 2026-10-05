//! Reproducible velocity/c bound audit; culling also expands from each frame's actual observer velocity.
use astroterm::astro::{
    COMPUTATIONAL_INTERVAL,
    models::{BodyId, planets::compute_barycentric_position},
};
fn main() {
    let span = COMPUTATIONAL_INTERVAL.end_tt - COMPUTATIONAL_INTERVAL.start_tt;
    let mut maximum = (0.0_f64, 0.0_f64);
    let count = 200000; // ~36.5-day spacing, covering orbital phases throughout 20,001 years
    for i in 0..=count {
        let tt = COMPUTATIONAL_INTERVAL.start_tt + span * i as f64 / count as f64;
        let h = 1.0 / 1024.0;
        let velocity = (compute_barycentric_position(BodyId::Earth, tt + h)
            - compute_barycentric_position(BodyId::Earth, tt - h))
            * (0.5 / h);
        let speed = velocity.length() + 0.4651011 * 86400.0 / 149597870.7; // maximum WGS84 equatorial spin, any site orientation
        let angle = (speed / 173.144632674240).asin();
        if angle > maximum.0 {
            maximum = (angle, tt);
        }
    }
    println!(
        "{} samples: maximum annual + worst-case diurnal aberration {:.6} arcsec at TT {:.6}; margin {:.1} arcsec",
        count + 1,
        maximum.0.to_degrees() * 3600.0,
        maximum.1,
        astroterm::model::grid::ABERRATION_MARGIN.to_degrees() * 3600.0
    );
    assert!(maximum.0 < astroterm::model::grid::ABERRATION_MARGIN);
}
