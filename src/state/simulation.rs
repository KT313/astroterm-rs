//! Reception/emission samples in TT days. Planet states are barycentric AU and AU/day;
//! lunar samples remain parent-relative. Family policies/versions control clearing independently.
use crate::{astro::{Matrix3, models::BodyState}, model::simulation::{Sample, CachePolicy, RefreshCounts, ModelFamily}};
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationState {
    pub(crate) planets: Vec<Sample<[BodyState; 9]>>, // reception/emission samples in BodyId order; bounded by request policy
    pub(crate) moon: Vec<Sample<BodyState>>,         // parent-relative lunar samples, composed with Earth at the requested time
    pub(crate) orientation: Vec<Sample<Matrix3>>,    // slow inertial-to-date rotation; Earth spin/site geometry remain observation work
    pub(crate) policy: CachePolicy,                // permitted TT half-spans; zero clears that family at frame start
    pub refresh_counts: RefreshCounts,             // cumulative evaluated sample blocks, used by observer cache keys
    pub(crate) versions: [u64; 3],                  // model identity; changes clear dependent families before the next request
}
impl SimulationState {
    /// Apply validated policies once at startup. A zero span keeps exact within-frame samples only.
    pub fn configure_cache(&mut self, config: &crate::cache::CacheConfig) {
        use crate::cache::Group;
        self.policy = CachePolicy {
            planets_days: config.age_seconds(Group::PlanetarySamples) / 86400.0,
            moon_days: config.age_seconds(Group::LunarSamples) / 86400.0,
            orientation_days: config.age_seconds(Group::SlowOrientation) / 86400.0,
        };
        self.planets.clear();
        self.moon.clear();
        self.orientation.clear();
    }
    /// Clear bypassed families once per frame, not between reception and emission requests.
    pub fn begin_frame(&mut self) {
        if self.policy.planets_days == 0.0 {
            self.planets.clear();
        }
        if self.policy.moon_days == 0.0 {
            self.moon.clear();
        }
        if self.policy.orientation_days == 0.0 {
            self.orientation.clear();
        }
    }
    pub fn model_versions(&self) -> [u64; 3] {
        self.versions
    }
    /// Direct per-frame evaluation, used as the reference for cache qualification.
    pub fn exact() -> Self {
        Self {
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
        match family {
            ModelFamily::Planets => self.planets.clear(),
            ModelFamily::Moon => self.moon.clear(),
            ModelFamily::Orientation => {
                self.orientation.clear();
                self.moon.clear();
            }
        }
    }
}
#[cfg(feature = "memory-diagnostics")]
crate::cache::buffers::report_fields!(SimulationState { planets, moon, orientation });
