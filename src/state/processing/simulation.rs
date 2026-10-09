//! Reception/emission samples in TT days. Planet states are barycentric AU and AU/day;
//! lunar samples remain parent-relative. Family policies/versions control clearing independently.
use crate::astro::Matrix3;
use crate::astro::models::BodyState;
use crate::model::{Sample, CachePolicy, RefreshCounts, ModelFamily};
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationState {
    pub(crate) identity: super::StageId,
    pub(crate) group: SolarGroup,                  // reception and emission preparation share this request lifecycle
    pub(crate) reuse: [bool; 3],                    // explicit planet/Moon/orientation reuse switches, separate from sample spans
    pub(crate) exact_mode: bool,                   // the direct-reference mode must refresh even while paused
    pub(crate) planet_work: Vec<Sample<BodyState>>,   // one work buffer serves all nine planetary lists in turn; it retains the displaced allocation
    pub(crate) moon_work: Vec<Sample<BodyState>>,
    pub(crate) orientation_work: Vec<Sample<Matrix3>>,
    pub(crate) planets: [Vec<Sample<BodyState>>; 9], // per body in BodyId order: reception and that body's emission samples; bounded by request policy
    pub(crate) moon: Vec<Sample<BodyState>>,         // parent-relative lunar samples, composed with Earth at the requested time
    pub(crate) orientation: Vec<Sample<Matrix3>>,    // slow inertial-to-date rotation; Earth spin/site geometry remain observation work
    pub(crate) policy: CachePolicy,                // permitted TT half-spans; zero means exact-time coverage, not disabled caching
    pub refresh_counts: RefreshCounts,             // cumulative evaluated sample blocks, used by observer cache keys
    pub(crate) versions: [u64; 3],                  // model identity; changes clear dependent families before the next request
}
impl SimulationState {
    /// Configure sample coverage and explicit reuse independently; old requests cannot survive a policy change.
    pub fn configure_cache(&mut self, config: &crate::cache::CacheConfig) {
        use crate::cache::Group;
        self.policy = CachePolicy {
            planets_days: config.age_seconds(Group::PlanetarySamples) / 86400.0,
            moon_days: config.age_seconds(Group::LunarSamples) / 86400.0,
            orientation_days: config.age_seconds(Group::SlowOrientation) / 86400.0,
        };
        self.reuse = [Group::PlanetarySamples, Group::LunarSamples, Group::SlowOrientation].map(|g| config.allows(g));
        self.exact_mode = false;
        self.invalidate();
        self.clear_samples();
    }
    /// Compatibility for standalone sampling callers. Managed frames use begin_solar_system_frame instead.
    pub fn begin_frame(&mut self) {
        if !self.reuse[0] { self.clear_planets(); }
        if !self.reuse[1] { self.moon.clear(); }
        if !self.reuse[2] { self.orientation.clear(); }
        if self.reuse.contains(&false) { self.invalidate(); }
    }
    /// Mark the complete group stale without freeing any allocated sample buffers.
    pub fn invalidate(&mut self) {
        self.group.has_been_invalidated = true;
        self.group.complete_key = None;
    }
    pub(crate) fn abort_request(&mut self) {
        self.invalidate();
        self.group.request_key = None;
    }
    fn clear_planets(&mut self) { for samples in &mut self.planets { samples.clear(); } }
    pub(crate) fn clear_samples(&mut self) {
        self.clear_planets();
        self.moon.clear();
        self.orientation.clear();
        self.planet_work.clear();
        self.moon_work.clear();
        self.orientation_work.clear();
    }
    pub(crate) fn request_key(&self, time: crate::model::FrameTime, site: crate::astro::Observer, observer_owner: u64) -> crate::model::SolarRequestKey {
        crate::model::SolarRequestKey { time, site, versions: self.versions, policy: self.policy, observer_owner }
    }
    pub(crate) fn request_token(&self) -> (super::StageId, u64) { (self.identity, self.group.request_generation) }
    pub(crate) fn start_request(&mut self, key: crate::model::SolarRequestKey) {
        self.invalidate();
        self.group.request_key = Some(key);
        self.group.request_generation = self.group.request_generation.checked_add(1).expect("solar request generation exhausted");
        self.clear_samples();
    }
    pub(crate) fn complete_request(&mut self, key: crate::model::SolarRequestKey) {
        assert_eq!(self.group.request_key, Some(key), "solar request changed during preparation");
        self.group.complete_key = Some(key);
        self.group.calculated_at = Some(key.time.tt);
        self.group.has_been_invalidated = false;
    }
    pub(crate) fn permits_reuse(&self) -> bool { !self.exact_mode && self.reuse.iter().all(|&enabled| enabled) }
    pub fn model_versions(&self) -> [u64; 3] {
        self.versions
    }
    /// Direct per-frame evaluation, used as the reference for cache qualification.
    pub fn exact() -> Self {
        Self {
            exact_mode: true, reuse: [false; 3],
            policy: CachePolicy {
                planets_days: 0.0,
                moon_days: 0.0,
                orientation_days: 0.0,
            },
            ..Self::default()
        }
    }
    pub fn set_model_version(&mut self, family: ModelFamily, version: u64) {
        let index = family as usize;
        if self.versions[index] == version {
            return;
        }
        self.versions[index] = version;
        self.invalidate();
        match family {
            ModelFamily::Planets => self.clear_planets(),
            ModelFamily::Moon => self.moon.clear(),
            ModelFamily::Orientation => {
                self.orientation.clear();
                self.moon.clear();
            }
        }
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(SimulationState { planets, moon, orientation, planet_work, moon_work, orientation_work, group });

/// Separate simulation families, grouped only for state inspection.
#[derive(Default)]
pub struct SimulationCaches {
    pub solar_system: SimulationState,
    pub stars: super::StellarSimulationState,
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(SimulationCaches { solar_system, stars });

/// A complete group is published only after final emission-time bodies are prepared.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SolarGroup {
    pub request_key: Option<crate::model::SolarRequestKey>,
    pub complete_key: Option<crate::model::SolarRequestKey>,
    pub calculated_at: Option<f64>,
    pub has_been_invalidated: bool,
    pub request_generation: u64,
}
impl Default for SolarGroup {
    fn default() -> Self { Self { request_key: None, complete_key: None, calculated_at: None, has_been_invalidated: true, request_generation: 0 } }
}
crate::rows::row_columns!(SolarGroup { request_key, complete_key, calculated_at, has_been_invalidated, request_generation });
#[cfg(feature = "memory-diagnostics")]
crate::cache::report_flat!(SolarGroup);

impl Default for SimulationState {
    fn default() -> Self {
        Self { identity: Default::default(), group: Default::default(), reuse: [true; 3], exact_mode: false,
            planets: Default::default(), moon: Vec::new(), orientation: Vec::new(), planet_work: Vec::new(), moon_work: Vec::new(), orientation_work: Vec::new(),
            policy: Default::default(), refresh_counts: Default::default(), versions: [0; 3] }
    }
}
