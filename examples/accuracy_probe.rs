//! Phase-6 model/correction probe. Reads TT/UT1 epochs from stdin; never an independent reference. Data and
//! stars go through the frame loop's steps 1 and 2 and the stellar motion model, airless.
use astroterm::formulas::{
    Observer, Vector3,
    BodyId, evaluate_moon, *, evaluate_planets, years_since_j2000,
};
use astroterm::model::{Data, View};


use std::io::{self, BufRead};
fn main() {
    let catalog = astroterm::sky::prepare_owned_catalog(astroterm::catalog::load_embedded_catalog(&mut astroterm::progress::Report::quiet()).unwrap(), &mut astroterm::progress::Report::quiet()).unwrap().catalog;
    let bodies = Data::default();
    let site = Observer {
        latitude: 42.3601_f64.to_radians(),
        longitude: -71.0589_f64.to_radians(),
    };
    let components = |v: Vector3| vec![v.x, v.y, v.z];
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let fields: Vec<f64> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
        let tt = fields[0];
        let ut1 = fields.get(1).copied().unwrap_or(tt);
        // Observe 59 s after the requested epoch, as the old sampled pipeline did for its composed comparisons.
        let frame = astroterm::formulas::AstronomicalTime { utc_jd: ut1 + 59.0 / 86400.0, ut1_jd: ut1 + 59.0 / 86400.0, tt_jd: tt + 59.0 / 86400.0 };
        let stamp_ms = astroterm::formulas::utc_jd_to_time_ms(frame.utc_jd).unwrap();
        let system = astroterm::solar_system::calculate_solar_system_at_tt(&bodies.solar_system_sun, &bodies.solar_system_planets, &bodies.solar_system_moons, stamp_ms, frame.tt_jd, &mut astroterm::time::StepTimes::default());
        let observation = astroterm::observation::calculate_observation_at_dates(&site, false, &View::default(), stamp_ms, &frame, system.earth, &mut astroterm::time::StepTimes::default());
        let states = evaluate_planets(tt);
        let moon = evaluate_moon(tt).add_parent(states[BodyId::Earth as usize].state());
        let raw: Vec<_> = states
            .into_iter()
            .map(astroterm::formulas::BodyMotion::state)
            .chain([moon])
            .map(|s| vec![s.position.x, s.position.y, s.position.z, s.velocity.x, s.velocity.y, s.velocity.z])
            .collect();
        let body: Vec<_> = bodies.solar_system_planets.planets.iter().zip(&system.planets)
            .filter(|(planet, _)| planet.id != BodyId::Earth)
            .map(|(planet, &position)| (planet.name, components(observation.apparent_direction(position))))
            .chain([("Moon", components(observation.apparent_direction(system.moon_position(0, observation.observer.position))))])
            .collect();
        let stars: Vec<_> = [7001, 5340]
            .map(|hr| {
                let star = catalog.stars.iter().find(|s| s.id.0 == hr).unwrap();
                let sample = star.motion.evaluate(years_since_j2000(frame.tt_jd), star.magnitude);
                components(observation.apparent_star_direction(sample.direction))
            })
            .into();
        use astroterm::formulas::{
            MOON_VALIDATED_INTERVAL, ObjectClass, PLANET_VALIDATED_INTERVAL, STAR_VALIDATED_INTERVAL,
            accuracy_target_arcseconds,
        };
        let coverage = [STAR_VALIDATED_INTERVAL, PLANET_VALIDATED_INTERVAL, MOON_VALIDATED_INTERVAL]
            .map(|r| r.is_some_and(|r| r.contains(frame.tt_jd)));
        let targets = [ObjectClass::Stars, ObjectClass::SunAndPlanets, ObjectClass::Moon]
            .map(|c| accuracy_target_arcseconds(c, frame.tt_jd));
        println!(
            "{}",
            serde_json::json!({"tt":tt,"ut1":ut1,"coverage":coverage,"targets":targets,"states":raw,"observed_tt":frame.tt_jd,"observed_ut1":frame.ut1_jd,"bodies":body,"stars":stars,
            "precession":compute_precession_matrix(tt).matrix().0,"slow":compute_slow_orientation(tt).0,
            "nutation":compute_nutation(tt),"eo":compute_mean_equation_of_origins(tt),"obliquity":compute_obliquity(tt)})
        );
    }
}
