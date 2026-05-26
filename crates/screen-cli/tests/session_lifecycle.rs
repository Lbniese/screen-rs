use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screen_testkit::{PtySize, PtyTestProcess};

#[test]
fn detached_session_lists_attaches_and_quits() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("life");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping detached session lifecycle test: Unix socket bind is not permitted");
        return;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY allocation not available on this system");
        return;
    }

    let session_name = "e2e";
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };

    let start = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf ready; sleep 20"),
        ],
        Duration::from_secs(3),
    )
    .expect("start command should return");

    assert_success("start", &start);

    let list = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(3),
    )
    .expect("list command should return");
    assert_contains("list stdout", &list.stdout, b"e2e");
    assert_contains("list stdout", &list.stdout, b".e2e");
    assert_contains("list stdout", &list.stdout, b"(Detached)");

    let attach = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-r"), OsStr::new(session_name)],
        Duration::from_secs(3),
    )
    .expect("attach command should return");
    assert_success("attach", &attach);
    assert_contains("attach stdout", &attach.stdout, b"ready");

    let after_attach = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(3),
    )
    .expect("post-attach list command should return");
    assert_contains(
        "post-attach list stdout",
        &after_attach.stdout,
        b"(Detached)",
    );

    let quit = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(3),
    )
    .expect("quit command should return");
    assert_success("quit", &quit);

    wait_until_no_sessions(&candidate, temp.path());
}

#[test]
fn interactive_attach_detaches_and_reattaches() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("interactive");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping interactive detach test: Unix socket bind is not permitted");
        return;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY allocation not available on this system");
        return;
    }

    let session_name = "interactive";
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };

    let start = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf ready; while :; do sleep 1; done"),
        ],
        Duration::from_secs(3),
    )
    .expect("start command should return");
    assert_success("start", &start);

    let envs = [
        (
            OsString::from("SCREENDIR"),
            temp.path().as_os_str().to_owned(),
        ),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut attach = PtyTestProcess::spawn_with_env(
        &candidate,
        [OsStr::new("-r"), OsStr::new(session_name)],
        envs,
        PtySize::new(80, 24),
    )
    .expect("spawn interactive attach under PTY");

    attach
        .read_until(b"ready", Duration::from_secs(3))
        .expect("interactive attach receives buffered output");
    attach.send(b"\x01d").expect("send default detach key");
    let status = attach
        .wait_or_kill(Duration::from_secs(3))
        .expect("attach exits after detach");
    assert!(status.success(), "attach exited with {status}");

    let after_detach = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(3),
    )
    .expect("post-detach list command should return");
    assert_contains(
        "post-detach list stdout",
        &after_detach.stdout,
        b"(Detached)",
    );

    let reattach = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-r"), OsStr::new(session_name)],
        Duration::from_secs(3),
    )
    .expect("reattach command should return");
    assert_success("reattach", &reattach);
    assert_contains("reattach stdout", &reattach.stdout, b"ready");

    let quit = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(3),
    )
    .expect("quit command should return");
    assert_success("quit", &quit);
    wait_until_no_sessions(&candidate, temp.path());
}

#[test]
fn attached_create_runs_command_and_exits() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("attached-create");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping attached create test: Unix socket bind is not permitted");
        return;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY allocation not available on this system");
        return;
    }

    let session_name = "attached-create";
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };
    let envs = [
        (
            OsString::from("SCREENDIR"),
            temp.path().as_os_str().to_owned(),
        ),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut create = PtyTestProcess::spawn_with_env(
        &candidate,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf attached-ready; sleep 1"),
        ],
        envs,
        PtySize::new(80, 24),
    )
    .expect("spawn attached create under PTY");

    create
        .read_until(b"attached-ready", Duration::from_secs(3))
        .expect("attached create receives child output");
    let status = create
        .wait_or_kill(Duration::from_secs(3))
        .expect("attached create exits after child exits");
    assert!(status.success(), "attached create exited with {status}");

    wait_until_no_sessions(&candidate, temp.path());
}

#[test]
fn attached_create_detaches_and_reattaches() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("attached-detach");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping attached create detach test: Unix socket bind is not permitted");
        return;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY allocation not available on this system");
        return;
    }

    let session_name = "attached-detach";
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };
    let envs = [
        (
            OsString::from("SCREENDIR"),
            temp.path().as_os_str().to_owned(),
        ),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut create = PtyTestProcess::spawn_with_env(
        &candidate,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf attached-ready; while :; do sleep 1; done"),
        ],
        envs,
        PtySize::new(80, 24),
    )
    .expect("spawn attached create under PTY");

    create
        .read_until(b"attached-ready", Duration::from_secs(3))
        .expect("attached create receives child output");
    create.send(b"\x01d").expect("send default detach key");
    let status = create
        .wait_or_kill(Duration::from_secs(3))
        .expect("attached create exits after detach");
    assert!(status.success(), "attached create exited with {status}");

    let after_detach = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(3),
    )
    .expect("post-detach list command should return");
    assert_contains(
        "post-detach list stdout",
        &after_detach.stdout,
        b"(Detached)",
    );

    let reattach = run_screen(
        &candidate,
        temp.path(),
        [OsStr::new("-r"), OsStr::new(session_name)],
        Duration::from_secs(3),
    )
    .expect("reattach command should return");
    assert_success("reattach", &reattach);
    assert_contains("reattach stdout", &reattach.stdout, b"attached-ready");

    let quit = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(3),
    )
    .expect("quit command should return");
    assert_success("quit", &quit);
    wait_until_no_sessions(&candidate, temp.path());
}

#[test]
fn attach_or_create_creates_missing_session() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("attach-or-create-new");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping attach-or-create create test: Unix socket bind is not permitted");
        return;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY allocation not available on this system");
        return;
    }

    let session_name = "attach-or-create-new";
    let shell_path = temp.path().join("rr-shell");
    fs::write(&shell_path, "#!/bin/sh\nprintf rr-ready; sleep 1\n").expect("write custom shell");
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o700))
        .expect("chmod custom shell");
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };
    let envs = [
        (
            OsString::from("SCREENDIR"),
            temp.path().as_os_str().to_owned(),
        ),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("SHELL"), shell_path.as_os_str().to_owned()),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut process = PtyTestProcess::spawn_with_env(
        &candidate,
        [OsStr::new("-R"), OsStr::new(session_name)],
        envs,
        PtySize::new(80, 24),
    )
    .expect("spawn attach-or-create under PTY");

    process
        .read_until(b"rr-ready", Duration::from_secs(3))
        .expect("attach-or-create receives shell output");
    let status = process
        .wait_or_kill(Duration::from_secs(3))
        .expect("attach-or-create exits after shell exits");
    assert!(status.success(), "attach-or-create exited with {status}");

    wait_until_no_sessions(&candidate, temp.path());
}

#[test]
fn attach_or_create_attaches_existing_session() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("attach-or-create-existing");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping attach-or-create attach test: Unix socket bind is not permitted");
        return;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY allocation not available on this system");
        return;
    }

    let session_name = "attach-or-create-existing";
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };
    let start = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf existing-ready; while :; do sleep 1; done"),
        ],
        Duration::from_secs(3),
    )
    .expect("start command should return");
    assert_success("start", &start);

    let envs = [
        (
            OsString::from("SCREENDIR"),
            temp.path().as_os_str().to_owned(),
        ),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut attach = PtyTestProcess::spawn_with_env(
        &candidate,
        [OsStr::new("-R"), OsStr::new(session_name)],
        envs,
        PtySize::new(80, 24),
    )
    .expect("spawn attach-or-create attach under PTY");

    attach
        .read_until(b"existing-ready", Duration::from_secs(3))
        .expect("attach-or-create receives existing output");
    attach.send(b"\x01d").expect("send default detach key");
    let status = attach
        .wait_or_kill(Duration::from_secs(3))
        .expect("attach-or-create exits after detach");
    assert!(status.success(), "attach-or-create exited with {status}");

    let quit = run_screen(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(3),
    )
    .expect("quit command should return");
    assert_success("quit", &quit);
    wait_until_no_sessions(&candidate, temp.path());
}

#[derive(Debug)]
struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl CommandOutput {}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        // Keep path short enough to fit Unix socket paths within SUN_LEN
        // (104 bytes on macOS). Socket path = <temp>/<dir>/<pid>.<session>
        let short = if name.len() > 12 { &name[..12] } else { name };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let path = temp_base().join(format!("sr-{short}-{nanos}"));
        fs::create_dir(&path).unwrap_or_else(|error| {
            panic!(
                "failed to create temporary directory {}: {error}",
                path.display()
            )
        });
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct SessionGuard {
    candidate: PathBuf,
    runtime: PathBuf,
    session_name: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = run_screen(
            &self.candidate,
            &self.runtime,
            [
                OsStr::new("-S"),
                OsStr::new(&self.session_name),
                OsStr::new("-X"),
                OsStr::new("quit"),
            ],
            Duration::from_secs(1),
        );
    }
}

fn run_screen<'a>(
    candidate: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<CommandOutput> {
    let mut child = Command::new(candidate)
        .args(args)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child.stdout.take().expect("stdout pipe should exist");
    let mut stderr = child.stderr.take().expect("stderr pipe should exist");
    let stdout_thread = thread::spawn(move || read_all(&mut stdout));
    let stderr_thread = thread::spawn(move || read_all(&mut stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "command timed out after {timeout:?}: {}",
                candidate.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = stdout_thread
        .join()
        .expect("stdout reader should not panic")?;
    let stderr = stderr_thread
        .join()
        .expect("stderr reader should not panic")?;
    Ok(CommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_all(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn assert_success(label: &str, output: &CommandOutput) {
    assert!(
        output.status.success(),
        "{label} failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_contains(label: &str, haystack: &[u8], needle: &[u8]) {
    assert!(
        haystack
            .windows(needle.len())
            .any(|window| window == needle),
        "{label} did not contain {:?}; actual:\n{}",
        String::from_utf8_lossy(needle),
        String::from_utf8_lossy(haystack)
    );
}

fn wait_until_no_sessions(candidate: &Path, runtime: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = run_screen(
            candidate,
            runtime,
            [OsStr::new("-ls")],
            Duration::from_secs(1),
        )
        .expect("list command should return while waiting for cleanup");

        if output
            .stdout
            .windows(b"No Sockets found".len())
            .any(|window| window == b"No Sockets found")
        {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "session socket was not cleaned up; last list output:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn temp_base() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
    }
}

fn unix_socket_bind_allowed(runtime: &Path) -> bool {
    let socket_path = runtime.join("bind-probe");
    match UnixListener::bind(&socket_path) {
        Ok(listener) => {
            drop(listener);
            let _ = fs::remove_file(socket_path);
            true
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => false,
        Err(error) => panic!(
            "unexpected Unix socket bind error at {}: {error}",
            socket_path.display()
        ),
    }
}
