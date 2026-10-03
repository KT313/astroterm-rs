//! Phase-6 model/correction probe. Reads TT/UT1 epochs from stdin; never an independent reference.
use astroterm::astro::{
    Observer,
    models::{BodyId, moons::evaluate_moon, orientation::*, planets::evaluate_planets},
};
use astroterm::sky::{FrameTime, SimulationState, Sky, SkyRegion, observe_sky, prepare_observation, update_simulation};
use astroterm::timing::StepTimes;
use std::io::{self, BufRead};
fn main() {
    let cat = astroterm::catalog::load_embedded_catalog().unwrap();
    let mut sky = Sky::from_catalog(&cat);
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let fields: Vec<f64> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
        let tt = fields[0];
        let ut1 = fields.get(1).copied().unwrap_or(tt);
        let time = FrameTime { utc: ut1, ut1, tt };
        let mut simulation = SimulationState::default();
        let mut times = StepTimes::default();
        update_simulation(&mut simulation, time, &[], &mut times).unwrap();
        let site = Observer {
            latitude: 42.3601_f64.to_radians(),
            longitude: -71.0589_f64.to_radians(),
        };
        // Offset the frame from the cached sample to include normal interpolation in composed comparisons.
        let frame = FrameTime {
            utc: ut1 + 59.0 / 86400.0,
            ut1: ut1 + 59.0 / 86400.0,
            tt: tt + 59.0 / 86400.0,
        };
        update_simulation(&mut simulation, frame, &[], &mut times).unwrap();
        let observer = prepare_observation(&mut simulation, frame, site).unwrap();
        observe_sky(
            &simulation,
            &observer,
            f64::INFINITY,
            false,
            SkyRegion::All,
            &mut sky,
            &mut times,
        )
        .unwrap();
        let states = evaluate_planets(tt);
        let moon = evaluate_moon(tt).add_parent(states[BodyId::Earth as usize]);
        let raw: Vec<_> = states
            .into_iter()
            .chain([moon])
            .map(|s| {
                vec![
                    s.position.x,
                    s.position.y,
                    s.position.z,
                    s.velocity.x,
                    s.velocity.y,
                    s.velocity.z,
                ]
            })
            .collect();
        let body: Vec<_> = sky
            .planets
            .iter()
            .map(|p| (p.kind.name(), vec![p.position.x, p.position.y, p.position.z]))
            .chain([(
                "Moon",
                vec![sky.moon.position.x, sky.moon.position.y, sky.moon.position.z],
            )])
            .collect();
        let stars: Vec<_> = [7001, 5340]
            .map(|hr| {
                let s = sky.stars.iter().find(|s| s.id.0 == hr).unwrap();
                vec![s.position.x, s.position.y, s.position.z]
            })
            .into();
        use astroterm::astro::{
            MOON_VALIDATED_INTERVAL, ObjectClass, PLANET_VALIDATED_INTERVAL, STAR_VALIDATED_INTERVAL,
            accuracy_target_arcseconds,
        };
        let coverage = [
            STAR_VALIDATED_INTERVAL,
            PLANET_VALIDATED_INTERVAL,
            MOON_VALIDATED_INTERVAL,
        ]
        .map(|r| r.is_some_and(|r| r.contains(frame.tt)));
        let targets = [ObjectClass::Stars, ObjectClass::SunAndPlanets, ObjectClass::Moon]
            .map(|c| accuracy_target_arcseconds(c, frame.tt));
        println!(
            "{}",
            serde_json::json!({"tt":tt,"ut1":ut1,"coverage":coverage,"targets":targets,"states":raw,"observed_tt":frame.tt,"observed_ut1":frame.ut1,"bodies":body,"stars":stars,
            "precession":compute_precession_matrix(tt).matrix().0,"slow":compute_slow_orientation(tt).0,
            "nutation":compute_nutation(tt),"eo":compute_mean_equation_of_origins(tt),"obliquity":compute_obliquity(tt)})
        );
    }
}
