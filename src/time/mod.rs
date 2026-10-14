//! Startup measurements are consumed into initialized timing state before the frame loop.
mod time_manager;

pub(crate) use time_manager::{StartupTiming, TimeManager};
