//! Integration tests: rg (redripgrep) interface matches ripgrep.
//! When ripgrep is on PATH, these tests assert rg forwards correctly.

use std::path::PathBuf;
use std::process::Command;

fn rrg_bin() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_BIN_EXE_rrg").unwrap_or_else(|_| "target/debug/rrg".into()))
}

/// Skip test if ripgrep is not installed (rg invokes ripgrep on PATH).
fn ripgrep_available() -> bool {
    Command::new("ripgrep")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn rg_version_exits_zero_and_prints_ripgrep() {
    if !ripgrep_available() {
        return;
    }
    let out = Command::new(rrg_bin())
        .arg("--version")
        .output()
        .expect("run rg --version");
    assert!(out.status.success(), "rg --version should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ripgrep") || stdout.contains("rg"),
        "output should mention ripgrep or rg, got: {}",
        stdout
    );
}

#[test]
fn rg_help_exits_zero() {
    if !ripgrep_available() {
        return;
    }
    let out = Command::new(rrg_bin())
        .arg("--help")
        .output()
        .expect("run rg --help");
    assert!(out.status.success(), "rg --help should exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.is_empty(), "help should not be empty");
}

#[test]
fn rg_pattern_matches_literal() {
    if !ripgrep_available() {
        return;
    }
    let rrg = rrg_bin();
    let out = Command::new(&rrg)
        .arg("redripgrep_test_literal")
        .arg(".")
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()))
        .output()
        .expect("run rg pattern");
    // May exit 0 (match) or 1 (no match) depending on repo contents; should not crash
    assert!(out.status.code().is_some());
    let code = out.status.code().unwrap();
    assert!(code == 0 || code == 1, "exit code should be 0 or 1, got {}", code);
}

#[test]
fn rg_e_pattern_interface() {
    if !ripgrep_available() {
        return;
    }
    let out = Command::new(rrg_bin())
        .args(["-e", "test", "--version"])
        .output()
        .expect("run rg -e test --version");
    assert!(out.status.success(), "rg -e test --version should succeed");
}
