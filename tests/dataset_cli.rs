//! Dataset errors and name/path precedence are resolved before opening a terminal or starting a download.
use std::{fs, process::Command};

#[test]
fn an_existing_bare_file_takes_precedence_over_the_named_download() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("athyg"), b"not,a,catalog\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_astroterm"))
        .current_dir(directory.path())
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .args(["--dataset", "athyg"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("dataset has no `ra` column"), "{error}");
    assert!(!error.contains("Downloading"));
    assert!(!directory.path().join("data").exists());
}

#[test]
fn unknown_names_and_explicit_missing_paths_have_distinct_errors() {
    let directory = tempfile::tempdir().unwrap();
    for (argument, expected) in [
        ("unknown-dataset", "Known names: athyg"),
        ("./missing.csv", "cannot read ./missing.csv"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_astroterm"))
            .current_dir(directory.path())
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("XDG_CACHE_HOME", directory.path().join("cache"))
            .args(["--dataset", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("Downloading"));
    }
}
