//! Basic session lifecycle integration tests.
//!
//! These tests exercise the full session lifecycle: create, list, attach,
//! detach, and clean up. They use isolated runtime directories and the
//! `SCREEN_RS_PARENT_PID` mechanism to prevent zombie daemons.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// How long to wait for a session daemon to start.
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for a session to appear in -ls output.
const LIST_TIMEOUT: Duration = Duration::from_secs(5);

fn screen_rs_path() -> PathBuf {
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

/// Create a fresh isolated runtime directory.
fn create_runtime_dir(prefix: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "screen-test-{}-{}",
        prefix,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let home = dir.join("home");
    let screen_dir = dir.join("screen");
    std::fs::create_dir_all(&home).expect("create home dir");
    std::fs::create_dir_all(&screen_dir).expect("create screen dir");
    (dir, home, screen_dir)
}

/// Run screen-rs and return (status, stdout, stderr).
fn run_screen_rs_with_env(
    args: &[&str],
    home: &PathBuf,
    screen_dir: &PathBuf,
) -> (std::process::ExitStatus, Vec<u8>, Vec<u8>) {
    let output = Command::new(screen_rs_path())
        .args(args)
        .env_remove("SCREEN_REFERENCE")
        .env("HOME", home)
        .env("SCREENDIR", screen_dir)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .env("SCREEN_RS_PARENT_PID", std::process::id().to_string())
        .output()
        .expect("failed to run screen-rs");
    (output.status, output.stdout, output.stderr)
}

/// Find the socket file in a screen directory.
fn find_socket(screen_dir: &PathBuf, session_name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(screen_dir).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.contains(session_name) {
            return Some(entry.path());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

#[test]
fn create_detached_session_child_survives() {
    let (_root, home, screen_dir) = create_runtime_dir("detached_survive");

    // Start a detached session running `sh -c 'sleep 30'`
    let (status, stdout, stderr) = run_screen_rs_with_env(
        &["-S", "survive-test", "-d", "-m", "sh", "-c", "sleep 30"],
        &home,
        &screen_dir,
    );
    // The launcher should exit but the daemon should start
    let err = String::from_utf8_lossy(&stderr);
    assert!(
        status.success() || err.contains("started"),
        "launch should exit successfully or report started: stderr={err:?} stdout={:?}",
        String::from_utf8_lossy(&stdout)
    );

    // Wait for the daemon to create the socket
    let deadline = Instant::now() + DAEMON_START_TIMEOUT;
    let socket = loop {
        if let Some(socket) = find_socket(&screen_dir, "survive-test") {
            break Some(socket);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    assert!(socket.is_some(), "daemon should create a socket within timeout");
    let socket = socket.unwrap();
    assert!(socket.exists(), "socket should exist at {socket:?}");

    // List sessions - the session should appear
    let (list_status, list_stdout, list_stderr) = run_screen_rs_with_env(
        &["-ls"],
        &home,
        &screen_dir,
    );
    let list_out = String::from_utf8_lossy(&list_stdout);
    assert!(
        list_status.success(),
        "-ls should succeed: stderr={}", String::from_utf8_lossy(&list_stderr)
    );
    assert!(
        list_out.contains("survive-test") || list_out.contains("survive"),
        "-ls should list our session: {list_out}"
    );

    // Cleanup: can't easily detach without PTY, but we can send shutdown
    // via -X quit
    let (_q_status, _q_stdout, q_stderr) = run_screen_rs_with_env(
        &["-S", "survive-test", "-X", "quit"],
        &home,
        &screen_dir,
    );
    let q_err = String::from_utf8_lossy(&q_stderr);
    if !q_err.is_empty() {
        eprintln!("quit stderr: {q_err}");
    }
}

#[test]
fn list_empty_directory() {
    let (_root, home, screen_dir) = create_runtime_dir("list_empty");

    let (status, stdout, stderr) = run_screen_rs_with_env(&["-ls"], &home, &screen_dir);
    assert!(
        status.success() || status.code() == Some(1),
        "-ls with empty dir should return GNU-compatible success/no-sockets status"
    );
    let out = String::from_utf8_lossy(&stdout);
    assert!(
        !out.is_empty(),
        "-ls should produce output even with no sessions: stderr={:?}",
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn wipe_empty_directory() {
    let (_root, home, screen_dir) = create_runtime_dir("wipe_empty");

    let (status, _, stderr) = run_screen_rs_with_env(&["-wipe"], &home, &screen_dir);
    assert!(
        status.success() || status.code() == Some(1),
        "-wipe with empty dir should return GNU-compatible success/no-sockets status: stderr={:?}",
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn create_then_wipe() {
    let (_root, home, screen_dir) = create_runtime_dir("create_wipe");

    // Create a short-lived detached session
    let (status, _, stderr) = run_screen_rs_with_env(
        &["-S", "wipe-test", "-d", "-m", "sh", "-c", "sleep 5"],
        &home,
        &screen_dir,
    );
    let err = String::from_utf8_lossy(&stderr);
    assert!(
        status.success() || err.contains("started"),
        "create should work: {err}"
    );

    // Wait briefly for the daemon to start
    std::thread::sleep(Duration::from_millis(500));

    // Wipe should clean up
    let (wipe_status, wipe_stdout, wipe_stderr) = run_screen_rs_with_env(
        &["-wipe"],
        &home,
        &screen_dir,
    );
    let wipe_err = String::from_utf8_lossy(&wipe_stderr);
    assert!(
        wipe_status.success(),
        "-wipe should succeed: {wipe_err}"
    );
    let _wipe_out = String::from_utf8_lossy(&wipe_stdout);
}

#[test]
fn attach_to_nonexistent_session_fails() {
    let (_root, home, screen_dir) = create_runtime_dir("attach_fail");

    let (status, _, stderr) = run_screen_rs_with_env(
        &["-r", "nonexistent"],
        &home,
        &screen_dir,
    );
    assert!(!status.success(), "attaching to non-existent session should fail");
}
