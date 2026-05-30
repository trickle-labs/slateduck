//! v0.46.0 — CLI hardening tests.
//!
//! Verifies that `rocklake --help` and all subcommand `--help` flags exit
//! with code 0 (not 1 as the old hand-rolled parser did).

use std::process::Command;

/// Return the path to the `rocklake` binary built in the test profile.
///
/// `cargo test` compiles the binary into `target/{profile}/rocklake`.
fn rocklake_bin() -> std::path::PathBuf {
    // CARGO_BIN_EXE_rocklake is set automatically by cargo when the crate
    // declares a `[[bin]]` with name = "rocklake".
    std::path::PathBuf::from(
        std::env::var("CARGO_BIN_EXE_rocklake")
            .unwrap_or_else(|_| "./target/debug/rocklake".to_string()),
    )
}

fn help_exits_zero(args: &[&str]) {
    let bin = rocklake_bin();
    if !bin.exists() {
        eprintln!("skipping {args:?}: binary not found at {bin:?}");
        return;
    }
    let output = Command::new(&bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {:?} with args {args:?}: {e}", bin));
    assert_eq!(
        output.status.code(),
        Some(0),
        "rocklake {args:?} exited with non-zero code.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn help_top_level() {
    help_exits_zero(&["--help"]);
}

#[test]
fn help_serve() {
    help_exits_zero(&["serve", "--help"]);
}

#[test]
fn help_gc_plan() {
    help_exits_zero(&["gc", "plan", "--help"]);
}

#[test]
fn help_gc_apply() {
    help_exits_zero(&["gc", "apply", "--help"]);
}

#[test]
fn help_excise_plan() {
    help_exits_zero(&["excise", "plan", "--help"]);
}

#[test]
fn help_checkpoint_create() {
    help_exits_zero(&["checkpoint", "create", "--help"]);
}

#[test]
fn help_checkpoint_list() {
    help_exits_zero(&["checkpoint", "list", "--help"]);
}

#[test]
fn help_checkpoint_restore() {
    help_exits_zero(&["checkpoint", "restore", "--help"]);
}

#[test]
fn help_export() {
    help_exits_zero(&["export", "--help"]);
}

#[test]
fn help_import() {
    help_exits_zero(&["import", "--help"]);
}

#[test]
fn help_rebuild() {
    help_exits_zero(&["rebuild", "--help"]);
}

#[test]
fn help_diagnose() {
    help_exits_zero(&["diagnose", "--help"]);
}

#[test]
fn help_sweep_orphans() {
    help_exits_zero(&["sweep-orphans", "--help"]);
}

#[test]
fn help_migrate_from_ducklake() {
    help_exits_zero(&["migrate-from-ducklake", "--help"]);
}

#[test]
fn help_export_catalog() {
    help_exits_zero(&["export-catalog", "--help"]);
}

#[test]
fn version_flag() {
    help_exits_zero(&["--version"]);
}

#[test]
fn completions_bash() {
    let bin = rocklake_bin();
    if !bin.exists() {
        eprintln!("skipping completions test: binary not found");
        return;
    }
    let output = Command::new(&bin)
        .args(["completions", "bash"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "rocklake completions bash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("rocklake"),
        "bash completion output should mention 'rocklake'"
    );
}
