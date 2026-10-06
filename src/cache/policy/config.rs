//! Validated, startup-only update policies. Durations are absolute simulated seconds from a sample epoch.
use serde::Deserialize;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    StellarState,
    PlanetarySamples,
    LunarSamples,
    SlowOrientation,
    ObserverState,
    SolarSystemObservation,
    CandidateSelection,
    WorkingSet,
    StellarVisibility,
    SolarSystemGeometry,
    ApparentDirections,
    HorizontalSky,
    Refraction,
    Projection,
    DrawOrder,
    ViewGeometry,
    Raster,
    RasterAssets,
}
impl Group {
    pub const ALL: [Self; 18] = [
        Self::StellarState,
        Self::PlanetarySamples,
        Self::LunarSamples,
        Self::SlowOrientation,
        Self::ObserverState,
        Self::SolarSystemObservation,
        Self::CandidateSelection,
        Self::WorkingSet,
        Self::StellarVisibility,
        Self::SolarSystemGeometry,
        Self::ApparentDirections,
        Self::HorizontalSky,
        Self::Refraction,
        Self::Projection,
        Self::DrawOrder,
        Self::ViewGeometry,
        Self::Raster,
        Self::RasterAssets,
    ];
    pub fn maximum_age(self) -> Option<f64> {
        match self {
            Self::StellarState => Some(360.0),
            Self::PlanetarySamples => Some(30.0),
            Self::LunarSamples => Some(12.0),
            Self::SlowOrientation => Some(60.0),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GroupPolicy {
    pub enabled: bool,
    pub max_age_seconds: Option<f64>,
}
impl Default for GroupPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_age_seconds: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
    pub version: u32,
    pub enabled: bool,
    pub groups: BTreeMap<Group, GroupPolicy>,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: true,
            groups: BTreeMap::new(),
        }
    }
}
impl CacheConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
    pub fn allows(&self, group: Group) -> bool {
        self.enabled
            && self
                .groups
                .get(&group)
                .is_none_or(|p| p.enabled && p.max_age_seconds != Some(0.0))
    }
    pub fn age_seconds(&self, group: Group) -> f64 {
        if !self.allows(group) {
            return 0.0;
        }
        self.groups
            .get(&group)
            .and_then(|p| p.max_age_seconds)
            .or(group.maximum_age())
            .unwrap_or(0.0)
    }
    pub fn parse(text: &str) -> io::Result<Self> {
        let config: Self = toml::from_str(text).map_err(io::Error::other)?;
        if config.version != 1 {
            return Err(io::Error::other("unsupported cache configuration version"));
        }
        for (&group, policy) in &config.groups {
            if let Some(age) = policy.max_age_seconds {
                let Some(maximum) = group.maximum_age() else {
                    return Err(io::Error::other(format!(
                        "{group:?} uses dependency invalidation, not a TTL"
                    )));
                };
                if !age.is_finite() || age < 0.0 || age > maximum {
                    return Err(io::Error::other(format!(
                        "{group:?} max_age_seconds must be finite and within 0..={maximum}"
                    )));
                }
            }
        }
        Ok(config)
    }
    pub fn load(explicit: Option<&Path>, disable: bool) -> io::Result<Self> {
        let default = dirs::config_dir().map(|p| p.join("astroterm/cache.toml"));
        let path = explicit.or(default.as_deref());
        let mut config = match path {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(text) => Self::parse(&text).map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?,
                Err(e) if explicit.is_none() && e.kind() == io::ErrorKind::NotFound => Self::default(),
                Err(e) => return Err(io::Error::other(format!("{}: {e}", path.display()))),
            },
            None => Self::default(),
        };
        if disable {
            config.enabled = false;
        }
        Ok(config)
    }
}

#[cfg(feature = "memory-diagnostics")]
crate::cache::report_fields!(CacheConfig { groups });

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_overrides_and_invalid_policies() {
        let c =
            CacheConfig::parse("[groups.stellar_state]\nmax_age_seconds=12\n[groups.raster]\nenabled=false").unwrap();
        assert_eq!(c.age_seconds(Group::StellarState), 12.0);
        assert_eq!(c.age_seconds(Group::PlanetarySamples), 30.0);
        assert!(!c.allows(Group::Raster));
        for text in [
            "version=2",
            "enabld=true",
            "[groups.unknown]",
            "[groups.raster]\nmax_age_seconds=1",
            "[groups.stellar_state]\nmax_age_seconds=nan",
            "[groups.stellar_state]\nmax_age_seconds=-1",
            "[groups.stellar_state]\nmax_age_seconds=361",
        ] {
            assert!(CacheConfig::parse(text).is_err(), "{text}");
        }
    }
    #[test]
    fn shipped_example_covers_all_groups_and_zero_disables_reuse() {
        let example = CacheConfig::parse(include_str!("../../../examples/cache.toml")).unwrap();
        for group in Group::ALL {
            assert!(example.groups.contains_key(&group));
        }
        let zero = CacheConfig::parse("[groups.stellar_state]\nmax_age_seconds=0").unwrap();
        assert!(!zero.allows(Group::StellarState));
    }

    #[test]
    fn explicit_file_and_cli_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.toml");
        assert!(CacheConfig::load(Some(&path), false).is_err());
        std::fs::write(&path, "enabled=true").unwrap();
        assert!(!CacheConfig::load(Some(&path), true).unwrap().enabled);
    }
}
