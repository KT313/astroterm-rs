//! Runtime processing caches. Persistent catalog files remain in `catalog::cache`.
mod config;
mod state;
pub use config::{CacheConfig, Group, GroupPolicy};
pub use state::{Cache, CacheStats, RefreshReason};

mod diagnostics;
pub use diagnostics::{CacheReport, format_stats};
