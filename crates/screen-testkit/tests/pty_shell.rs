use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use screen_pty::{PtyCommand, PtyProcess, PtySize};
use screen_testkit::PtyTestProcess;

macro_rules! skip_if_no_pty {
    () => {
        if !screen_pty::pty_available() {
            eprintln!("skipping PTY test: PTY allocation not available on this system");
            return;
        }
    };
}

#[test]
fn shell_can_be_driven_through_pty() {
    skip_if_no_pty!();
    let mut process =
        PtyProcess::spawn("/bin/sh", std::iter::empty::<&str>(), PtySize::new(80, 24))
            .expect("spawn shell through PTY");

    process
        .write_all(b"printf 'ready\\n'\n")
        .expect("write command to PTY");
    let output = process
        .read_until(b"ready", Duration::from_secs(5))
        .expect("observe command output");
    assert!(
        output
            .windows(b"ready".len())
            .any(|window| window == b"ready"),
        "PTY output did not contain expected bytes: {output:?}"
    );

    // Drain remaining output before resize
    let _ = process.read_available();

    process
        .resize(PtySize::new(100, 40))
        .expect("resize shell PTY");
    std::thread::sleep(Duration::from_millis(100));
    // Drain any resize-related output
    let _ = process.read_available();

    process
        .write_all(b"exit\n")
        .expect("write exit command to PTY");

    // Give the shell time to process
    std::thread::sleep(Duration::from_millis(200));
    let _drain = process.read_available();

    let status = process
        .wait_or_kill(Duration::from_secs(5))
        .expect("shell exits before timeout");

    assert!(status.success(), "shell exited with {status}");
}

#[test]
fn resize_delivers_sigwinch_to_child() {
    skip_if_no_pty!();
    let mut shell = PtyTestProcess::spawn(
        "/bin/sh",
        [
            "-c",
            "trap 'printf resized' WINCH; printf ready; while :; do sleep 1; done",
        ],
        PtySize::new(80, 24),
    )
    .expect("spawn shell through PTY");

    shell
        .read_until(b"ready", Duration::from_secs(2))
        .expect("observe initial output");
    shell
        .resize(PtySize::new(100, 40))
        .expect("resize shell PTY");
    let output = shell
        .read_until(b"resized", Duration::from_secs(2))
        .expect("observe resize trap output");

    assert!(
        output
            .windows(b"resized".len())
            .any(|window| window == b"resized"),
        "PTY output did not contain resize marker: {output:?}"
    );

    let _ = shell.wait_or_kill(Duration::from_millis(100));
}

#[test]
fn pty_command_honors_current_dir() {
    skip_if_no_pty!();
    let temp = std::env::temp_dir().join(format!(
        "screen-rs-pty-cwd-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&temp).expect("create temp cwd");

    let mut command = PtyCommand::new("/bin/sh", PtySize::new(80, 24));
    command.args(["-c", "pwd"]);
    command.current_dir(&temp);
    let mut process = command.spawn().expect("spawn pwd in temp cwd");
    let output = process
        .read_until(temp.to_string_lossy().as_bytes(), Duration::from_secs(2))
        .expect("observe cwd output");
    let status = process
        .wait_or_kill(Duration::from_secs(2))
        .expect("pwd exits");
    let _ = fs::remove_dir(&temp);

    assert!(status.success(), "pwd exited with {status}");
    assert!(
        output
            .windows(temp.to_string_lossy().len())
            .any(|window| window == temp.to_string_lossy().as_bytes()),
        "PTY output did not contain cwd: {}",
        String::from_utf8_lossy(&output)
    );
}
