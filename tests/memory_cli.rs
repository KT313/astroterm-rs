//! Memory diagnostics have independent build, runtime and on-screen timing controls.
use astroterm::cli::{Arguments, build_config, write_bash_completions};
use clap::{CommandFactory, Parser};

#[test]
fn memory_activation_has_an_explicit_build_and_runtime_contract() {
    let plain = build_config(Arguments::try_parse_from(["astroterm"]).unwrap(), &[]).unwrap();
    assert!(!plain.debug_memory);
    for flags in [vec!["astroterm", "--debug-memory"], vec!["astroterm", "--debug-memory", "--debug-singleframe"]] {
        let result = build_config(Arguments::try_parse_from(flags).unwrap(), &[]);
        #[cfg(not(feature = "memory-diagnostics"))]
        assert!(result.unwrap_err().to_string().contains("cargo build --features memory-diagnostics"));
        #[cfg(feature = "memory-diagnostics")]
        {
            let config = result.unwrap();
            assert!(config.debug_memory);
            assert!(!config.terminal.metadata_panel);
            assert!(!config.terminal.frame_times);
        }
    }
}

#[test]
fn memory_help_and_completions_describe_the_control() {
    let help = Arguments::command().render_long_help().to_string().split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(help.contains("after quitting"));
    assert!(help.contains("cargo build --features memory-diagnostics --bin astroterm"));
    let mut completions = Vec::new();
    write_bash_completions(&mut completions, &[]).unwrap();
    assert!(String::from_utf8(completions).unwrap().contains("--debug-memory"));
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn memory_keeps_timing_and_cache_switches_independent() {
    let config = build_config(Arguments::try_parse_from([
        "astroterm", "--debug-memory", "--debug-frametimes", "--disable-cache",
    ]).unwrap(), &[]).unwrap();
    assert!(config.terminal.frame_times);
    assert!(config.terminal.metadata_panel);
    assert!(!config.cache.enabled);
    assert!(!config.debug_singleframe);
}

#[cfg(not(feature = "memory-diagnostics"))]
#[test]
fn unsupported_memory_mode_exits_before_alternate_screen() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_astroterm")).arg("--debug-memory").output().unwrap();
    assert!(!output.status.success());
    assert!(!output.stdout.windows(8).any(|w| w == b"\x1b[?1049h"));
    assert!(String::from_utf8(output.stderr).unwrap().contains("cargo build --features memory-diagnostics"));
}

#[cfg(feature = "memory-diagnostics")]
#[test]
fn catalog_failure_prints_startup_report_without_terminal_takeover() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_astroterm"))
        .args(["--debug-memory", "--dataset", "/nonexistent/astroterm-s6-missing-catalog.csv"])
        .output().unwrap();
    assert!(!output.status.success());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains("astroterm --debug-memory: run report"));
    assert!(report.contains("City loading"));
    assert!(report.contains("Dataset loading"));
    assert!(!report.contains("Incomplete frame"));
    assert!(!report.contains("\x1b[?1049h"));
    assert!(String::from_utf8(output.stderr).unwrap().contains("ERROR:"));
}
