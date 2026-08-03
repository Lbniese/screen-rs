use std::path::PathBuf;
use std::time::Duration;

use screen_pty::{PtyCommand, PtyProcess, PtySize};

const SIZE: PtySize = PtySize::new(80, 24);
const TIMEOUT: Duration = Duration::from_secs(5);

fn command_path(name: &str, fallback: &str) -> Option<PathBuf> {
    let path = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| PathBuf::from(path.trim()))
        .filter(|path| !path.as_os_str().is_empty());
    path.or_else(|| {
        let fallback = PathBuf::from(fallback);
        fallback.is_file().then_some(fallback)
    })
}

#[test]
fn spawn_echo_and_read_output() {
    let Some(echo) = command_path("echo", "/bin/echo") else {
        return;
    };
    let mut pty = PtyProcess::spawn(echo, ["hello-pty"], SIZE).expect("spawn echo");
    let output = pty
        .read_until(b"hello-pty", TIMEOUT)
        .expect("echo output within timeout");
    assert!(
        output
            .windows(b"hello-pty".len())
            .any(|window| window == b"hello-pty")
    );
    assert!(pty.wait_or_kill(TIMEOUT).expect("echo exits").success());
}

#[test]
fn spawn_with_env_and_current_dir() {
    let Some(sh) = command_path("sh", "/bin/sh") else {
        return;
    };
    let requested_dir = std::env::temp_dir();
    // Canonicalize so platform quirks (macOS `/var` -> `/private/var` symlinks,
    // or a TMPDIR trailing slash) do not make the working-directory check flaky.
    let canonical = requested_dir
        .canonicalize()
        .unwrap_or_else(|_| requested_dir.clone());
    let canonical_bytes = canonical.to_string_lossy().into_owned();
    // Print the physical CWD first, then the env var, so a single `read_until`
    // captures both and proves `current_dir` and `env` both took effect.
    let mut command = PtyCommand::new(sh, SIZE);
    command
        .args(["-c", "pwd -P; echo \"SCREEN_PTY_TEST=$SCREEN_PTY_TEST\""])
        .env("SCREEN_PTY_TEST", "present")
        .current_dir(&requested_dir);
    let mut pty = command.spawn().expect("spawn sh");
    let output = pty
        .read_until(b"SCREEN_PTY_TEST=present", TIMEOUT)
        .expect("env marker within timeout");
    assert!(
        output
            .windows(canonical_bytes.len())
            .any(|window| window == canonical_bytes.as_bytes()),
        "current_dir {:?} not reflected by `pwd -P`; output was {:?}",
        requested_dir,
        String::from_utf8_lossy(&output),
    );
    assert!(pty.wait_or_kill(TIMEOUT).expect("sh exits").success());
}

#[test]
fn resize_updates_pty_without_error() {
    let Some(sh) = command_path("sh", "/bin/sh") else {
        return;
    };
    let mut pty = PtyProcess::spawn(sh, ["-c", "sleep 1"], SIZE).expect("spawn shell");
    pty.resize(PtySize::new(120, 40)).expect("resize PTY");
    assert!(pty.wait_or_kill(TIMEOUT).expect("shell exits").success());
}

#[test]
fn read_available_returns_ok_after_child_exit() {
    let Some(true_path) = command_path("true", "/usr/bin/true") else {
        return;
    };
    let mut pty =
        PtyProcess::spawn(true_path, std::iter::empty::<&str>(), SIZE).expect("spawn true");
    assert!(pty.wait_or_kill(TIMEOUT).expect("true exits").success());
    pty.read_available().expect("child exit is EOF");
}
