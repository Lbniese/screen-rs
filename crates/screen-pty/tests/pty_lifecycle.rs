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
    let Some(env) = command_path("env", "/usr/bin/env") else {
        return;
    };
    let expected_dir = std::env::temp_dir();
    let mut command = PtyCommand::new(env, SIZE);
    command
        .env("SCREEN_PTY_TEST", "present")
        .current_dir(&expected_dir);
    let mut pty = command.spawn().expect("spawn env");
    let output = pty
        .read_until(b"SCREEN_PTY_TEST=present", TIMEOUT)
        .expect("environment output within timeout");
    assert!(
        output
            .windows(expected_dir.to_string_lossy().len())
            .any(|window| { window == expected_dir.to_string_lossy().as_bytes() })
    );
    assert!(pty.wait_or_kill(TIMEOUT).expect("env exits").success());
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
