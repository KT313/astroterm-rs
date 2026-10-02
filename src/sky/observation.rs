//! Observer-dependent transformations and corrections. No View, ephemeris evaluation, or cache mutation.
use super::{
    FrameTime, ObservedSky, ObservedStar, PlanetKind, SimulationError, SimulationState, refract_sky_positions,
};
use crate::astro::models::{
    BodyId, BodyState,
    moons::EARTH_RADIUS_AU,
    orientation::{compute_body_fixed_rotation, compute_horizon_rotation},
};
use crate::astro::{Horizontal, Matrix3, Observer, compute_star_position, correct_for_parallax};
use crate::timing::StepTimes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Earth,
}

/// A frame's observer in the common inertial frame. Site coordinates are body-fixed; full orientation (slow and
/// fast) transforms a site's vector and velocity. Production uses zero site displacement until exact parallax.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObserverState {
    pub anchor: Anchor,
    pub site: Observer,
    pub height_m: f64,
    pub time: FrameTime,
    pub state: BodyState,
    pub inertial_to_fixed: Matrix3,
    pub inertial_to_horizon: Matrix3,
    pub atmosphere: bool,
    pub legacy_lunar_parallax: bool,
}
impl ObserverState {
    /// Compose a body's translation and complete orientation with a body-fixed site state. This geometry also
    /// serves synthetic anchors in tests; it assumes neither an Earth orbit nor spin around inertial Z.
    pub fn from_anchor_state(
        time: FrameTime,
        site: Observer,
        anchor_state: BodyState,
        inertial_to_fixed: Matrix3,
        site_fixed: BodyState,
        atmosphere: bool,
    ) -> Self {
        let fixed_to_inertial = inertial_to_fixed.transpose();
        let offset = BodyState {
            position: fixed_to_inertial.apply(site_fixed.position),
            velocity: fixed_to_inertial.apply(site_fixed.velocity),
        };
        Self {
            anchor: Anchor::Earth,
            site,
            height_m: 0.0,
            time,
            state: offset.add_parent(anchor_state),
            inertial_to_fixed,
            inertial_to_horizon: compute_horizon_rotation(&site).compose(inertial_to_fixed),
            atmosphere,
            legacy_lunar_parallax: false,
        }
    }
}

pub fn prepare_observer(
    simulation: &SimulationState,
    time: FrameTime,
    site: Observer,
) -> Result<ObserverState, SimulationError> {
    let earth = simulation.evaluate_body(BodyId::Earth, time.tt)?;
    let slow = simulation.evaluate_orientation(time.tt)?;
    let orientation = compute_body_fixed_rotation(slow, time.ut1);
    let mut observer = ObserverState::from_anchor_state(time, site, earth, orientation, BodyState::default(), true);
    observer.legacy_lunar_parallax = true;
    Ok(observer)
}

/// Evaluate the brightness-selected catalog prefix and every body at the observer's frame time. Refraction is part
/// of observation, applied once. The whole sky is the region until spatial indexing arrives in phase 4.
pub fn observe_sky(
    simulation: &SimulationState,
    observer: &ObserverState,
    magnitude_threshold: f32,
    refraction: bool,
    output: &mut ObservedSky,
    times: &mut StepTimes,
) -> Result<(), SimulationError> {
    // resolve all body dependencies before writing output, so missing coverage is explicit
    let tt = observer.time.tt;
    let bodies = PlanetKind::ALL
        .map(|kind| simulation.evaluate_body(body_id(kind), tt))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let moon = simulation.evaluate_body(BodyId::Moon, tt)?;
    let relative_moon = moon.position - observer.state.position;
    let relative_sun = bodies[0].position - observer.state.position;

    // stars need no finite-distance site correction yet
    times.measure("Stellar observation", || {
        let count = output.catalog.count_bright_stars(magnitude_threshold);
        output.stars.clear();
        output.stars.extend(output.catalog.stars[..count].iter().map(|star| {
            let direction = compute_star_position(star.catalog_position, star.proper_motion, tt).to_unit_vector();
            ObservedStar::from_star(
                star,
                Horizontal::from_vector(observer.inertial_to_horizon.apply(direction)),
            )
        }));
    });

    // finite bodies share subtraction and rotation; keep the inherited lunar parallax until the site upgrade
    times.measure("Body observation", || {
        for (planet, state) in output.planets.iter_mut().zip(bodies) {
            planet.position = Horizontal::from_vector(
                observer
                    .inertial_to_horizon
                    .apply(state.position - observer.state.position),
            );
        }
        output.moon.position = Horizontal::from_vector(observer.inertial_to_horizon.apply(relative_moon));
        if observer.legacy_lunar_parallax {
            output.moon.position = correct_for_parallax(output.moon.position, relative_moon.length() / EARTH_RADIUS_AU);
        }
        output.moon.illumination = super::compute_moon_illumination(
            relative_moon,
            relative_sun,
            crate::astro::models::orientation::j2000_ecliptic_north(),
        );
        output.moon.phase = output.moon.illumination.named_phase();
    });
    output.refracted = false;
    if refraction && observer.atmosphere {
        times.measure("Refraction", || refract_sky_positions(output));
    }
    Ok(())
}

fn body_id(kind: PlanetKind) -> BodyId {
    match kind {
        PlanetKind::Sun => BodyId::Sun,
        PlanetKind::Mercury => BodyId::Mercury,
        PlanetKind::Venus => BodyId::Venus,
        PlanetKind::Mars => BodyId::Mars,
        PlanetKind::Jupiter => BodyId::Jupiter,
        PlanetKind::Saturn => BodyId::Saturn,
        PlanetKind::Uranus => BodyId::Uranus,
        PlanetKind::Neptune => BodyId::Neptune,
    }
}
