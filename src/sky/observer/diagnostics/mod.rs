use crate::{model::FrameTime, state::ObserverPreparationCache, timing::StepTimes};
/// Keep diagnostic formatting lazy and outside the observer calculation's timer.
pub(super) fn describe_geometry(time: FrameTime, site: crate::astro::Observer, times: &mut StepTimes) {
    times.describe("Observer geometry", || format!("UTC JD={:.9}; UT1 JD={:.9}; TT JD={:.9}; latitude={} rad; longitude={} rad; output WGS84 observer state + horizon matrix", time.utc, time.ut1, time.tt, site.latitude, site.longitude));
}

/// Describe the completed light-time pass, preserving the original detail order and target steps.
pub(super) fn describe_emissions(observer: &crate::model::ObserverState, cache: &ObserverPreparationCache, times: &mut StepTimes) {
    times.describe("Light-time sampling", || format!("solar-system emission epochs={:?}; stars have no light-time iteration", observer.emission_tt));
    times.describe("Observer geometry", || format!("cache={:?}", cache.observer_report()));
    times.describe("Light-time sampling", || format!("cache={:?}", cache.light_time_report()));
}
