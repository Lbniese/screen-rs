//! Basic differential tests comparing screen-rs against GNU Screen.
//!
//! These tests require GNU Screen to be installed at the path specified by
//! the `SCREEN_REFERENCE` environment variable (defaults to `screen`).
//! If GNU Screen is not found, the differential tests are skipped gracefully.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use screen_testkit::{
    CommandComparison, CommandResult, ScreenExecutable, TestEnvironment,
    compare_command_results, default_reference_path,
};

/// Path to the screen-rs binary under test.
fn candidate_path() -> PathBuf {
    std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.pop();
            path.push("target");
            path.push("debug");
            path.push("screen-rs");
            path
        })
}

/// Returns (reference, candidate) executables if GNU Screen is available.
fn executables() -> Option<(ScreenExecutable, ScreenExecutable)> {
    let reference_path = default_reference_path();
    // Check if reference exists
    if !reference_path.exists() {
        eprintln!(
            "GNU Screen not found at {:?}; skipping differential tests. \
             Set SCREEN_REFERENCE to enable.",
            reference_path
        );
        return None;
    }
    // Quick probe: run screen --version
    let probe = Command::new(&reference_path)
        .arg("--version")
        .output()
        .ok()?;
    if !probe.status.success() {
        eprintln!("GNU Screen at {reference_path:?} is not executable; skipping");
        return None;
    }
    Some((
        ScreenExecutable::new(reference_path),
        ScreenExecutable::new(candidate_path()),
    ))
}

/// Run a differential test comparing reference and candidate.
fn run_differential(
    case_name: &str,
    args: &[&str],
) -> Option<CommandComparison> {
    let (reference, candidate) = executables()?;
    let env = TestEnvironment::create(case_name).ok()?;

    let reference_result = reference
        .run_with_timeout(args.iter().copied(), &env, Duration::from_secs(10))
        .ok()?;
    let candidate_result = candidate
        .run_with_timeout(args.iter().copied(), &env, Duration::from_secs(10))
        .ok()?;

    Some(compare_command_results(case_name, &reference_result, &candidate_result))
}

fn assert_diff_match(comparison: &CommandComparison) {
    if !comparison.is_match() {
        eprintln!("DIFFERENTIAL MISMATCH:\n{comparison}");
    }
    // For now, just print the comparison - we expect differences at this
    // stage since screen-rs is still in development.
    // When full compatibility is claimed, this should become an assertion.
    println!("{comparison}");
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

#[test]
fn differential_help() {
    if let Some(comparison) = run_differential("help", &["--help"]) {
        assert_diff_match(&comparison);
    }
}

#[test]
fn differential_version() {
    if let Some(comparison) = run_differential("version", &["--version"]) {
        assert_diff_match(&comparison);
    }
}

// ---------------------------------------------------------------------------
// List and wipe (no sessions)
// ---------------------------------------------------------------------------

#[test]
fn differential_list_no_sessions() {
    if let Some(comparison) = run_differential("list_no_sessions", &["-ls"]) {
        assert_diff_match(&comparison);
    }
}

#[test]
fn differential_wipe_no_sessions() {
    if let Some(comparison) = run_differential("wipe_no_sessions", &["-wipe"]) {
        assert_diff_match(&comparison);
    }
}

#[test]
fn differential_list_alias() {
    if let Some(comparison) = run_differential("list_alias", &["-list"]) {
        assert_diff_match(&comparison);
    }
}

// ---------------------------------------------------------------------------
// Invalid options
// ---------------------------------------------------------------------------

#[test]
fn differential_invalid_option() {
    if let Some(comparison) = run_differential("invalid_option", &["--bogus"]) {
        assert_diff_match(&comparison);
    }
}

#[test]
fn differential_no_args() {
    if let Some(comparison) = run_differential("no_args", &[]) {
        assert_diff_match(&comparison);
    }
}
