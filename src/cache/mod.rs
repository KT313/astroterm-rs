//! Runtime processing caches and buffer-reporting contracts. Persistent catalog files remain in `catalog::cache`.

#[path = "policy/config.rs"]
mod config;
#[path = "storage/state.rs"]
mod state;
#[path = "reporting/diagnostics.rs"]
mod diagnostics;

pub use config::{CacheConfig, Group, GroupPolicy};
pub use state::{Cache, CacheStats, RefreshReason, StoreOutcome, adopt_work, rewrite_in_place};
pub use diagnostics::{CacheReport, Quality, format_stats};
