//! Command line interface: argument definitions, their validation, and shell completions.

mod arguments;
mod completions;
mod config;

pub use arguments::Arguments;
pub use completions::write_bash_completions;
pub use config::{ConfigError, build_config, parse_azimuth};
