//! Runtime processing caches. Persistent catalog files remain in `catalog::cache`.
mod config;
mod state;
pub use config::{CacheConfig, Group, GroupPolicy};
pub use state::{Cache, CacheStats, RefreshReason, StoreOutcome};

mod diagnostics;
pub use diagnostics::{CacheReport, Quality, format_stats};

#[cfg(feature = "memory-diagnostics")]
pub mod buffers;
