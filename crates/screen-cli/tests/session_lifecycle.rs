use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
fn password_protected_attach_prompts_and_accepts_password() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("password-attach");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping password attach test: Unix socket bind is not permitted");
        return;
    }

    let session_name = "pwattach";
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
    assert!(
        wait_until_session_present(&candidate, temp.path(), session_name),
        "session should become visible before setting password"
    );
    set_session_password(temp.path(), session_name, b"secret");

    let denied = run_screen_with_input(
        &candidate,
        temp.path(),
        [OsStr::new("-r"), OsStr::new(session_name)],
        b"wrong\n",
        Duration::from_secs(3),
    )
    .expect("attach with wrong password should return");
    assert!(
        !denied.status.success(),
        "attach with wrong password unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&denied.stdout),
        String::from_utf8_lossy(&denied.stderr)
    );
    assert_contains("wrong-password stderr", &denied.stderr, b"Screen password: ");
    assert_contains(
        "wrong-password stderr",
        &denied.stderr,
        b"incorrect password",
    );

    let attach = run_screen_with_input(
        &candidate,
        temp.path(),
        [OsStr::new("-r"), OsStr::new(session_name)],
        b"secret\n",
        Duration::from_secs(3),
    )
    .expect("attach with correct password should return");
    assert_success("attach with password", &attach);
    assert_contains("attach stderr", &attach.stderr, b"Screen password: ");
    assert_contains("attach stdout", &attach.stdout, b"ready");

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
fn detached_session_exits_when_parent_pid_disappears() {
    let candidate = PathBuf::from(env!("CARGO_BIN_EXE_screen-rs"));
    let temp = TempDir::new("parent-pid");
    if !unix_socket_bind_allowed(temp.path()) {
        eprintln!("skipping parent-pid cleanup test: Unix socket bind is not permitted");
        return;
    }

    let session_name = "parent-pid";
    let _guard = SessionGuard {
        candidate: candidate.clone(),
        runtime: temp.path().to_owned(),
        session_name: session_name.to_owned(),
    };

    let mut parent = Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn parent pid sentinel");

    let start = run_screen_with_env(
        &candidate,
        temp.path(),
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("while :; do sleep 1; done"),
        ],
        &[(
            OsString::from("SCREEN_RS_PARENT_PID"),
            OsString::from(parent.id().to_string()),
        )],
        Duration::from_secs(3),
    )
    .expect("start command should return");
    assert_success("start", &start);
    assert!(
        wait_until_session_present(&candidate, temp.path(), session_name),
        "session should become visible before parent exits"
    );

    let _ = parent.kill();
    let _ = parent.wait();

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

static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
thread_local! {
    static TEST_LOCK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TEST_LOCK_GUARD: std::cell::RefCell<Option<std::sync::MutexGuard<'static, ()>>> =
        const { std::cell::RefCell::new(None) };
}

fn acquire_test_lock() {
    TEST_LOCK_DEPTH.with(|depth| {
        let current = depth.get();
        if current == 0 {
            TEST_LOCK_GUARD.with(|guard| {
                *guard.borrow_mut() = Some(
                    TEST_MUTEX
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
            });
        }
        depth.set(current + 1);
    });
}

fn release_test_lock() {
    TEST_LOCK_DEPTH.with(|depth| {
        let current = depth.get();
        if current <= 1 {
            TEST_LOCK_GUARD.with(|guard| {
                guard.borrow_mut().take();
            });
            depth.set(0);
        } else {
            depth.set(current - 1);
        }
    });
}

impl TempDir {
    fn new(name: &str) -> Self {
        acquire_test_lock();
        // Keep path short enough to fit Unix socket paths within SUN_LEN
        // (104 bytes on macOS). Socket path = <temp>/<dir>/<pid>.<session>
        let short = if name.len() > 12 { &name[..12] } else { name };
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = temp_base().join(format!("sr-{short}-{}-{seq}", std::process::id()));
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
        release_test_lock();
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
    run_screen_custom(candidate, runtime, args, &[], None, timeout)
}

fn run_screen_with_input<'a>(
    candidate: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    input: &[u8],
    timeout: Duration,
) -> io::Result<CommandOutput> {
    run_screen_custom(candidate, runtime, args, &[], Some(input), timeout)
}

fn run_screen_with_env<'a>(
    candidate: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    extra_envs: &[(OsString, OsString)],
    timeout: Duration,
) -> io::Result<CommandOutput> {
    run_screen_custom(candidate, runtime, args, extra_envs, None, timeout)
}

fn run_screen_custom<'a>(
    candidate: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    extra_envs: &[(OsString, OsString)],
    input: Option<&[u8]>,
    timeout: Duration,
) -> io::Result<CommandOutput> {
    let home = ensure_test_home(runtime);
    let mut command = Command::new(candidate);
    command
        .args(args)
        .env("HOME", &home)
        .env("ZDOTDIR", &home)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_envs {
        command.env(key, value);
    }

    let mut child = command.spawn()?;
    if let Some(input) = input {
        let mut stdin = child.stdin.take().expect("stdin pipe should exist");
        stdin.write_all(input)?;
        drop(stdin);
    }

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

fn ensure_test_home(runtime: &Path) -> PathBuf {
    let home = runtime.join("home");
    let _ = fs::create_dir_all(&home);
    home
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

fn wait_until_session_present(candidate: &Path, runtime: &Path, session_name: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = run_screen(
            candidate,
            runtime,
            [OsStr::new("-ls")],
            Duration::from_secs(1),
        )
        .expect("list command should return while waiting for session");

        if output
            .stdout
            .windows(session_name.len())
            .any(|window| window == session_name.as_bytes())
        {
            return true;
        }

        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn session_socket_path(runtime: &Path, session_name: &str) -> Option<PathBuf> {
    let suffix = format!(".{session_name}");
    let entries = fs::read_dir(runtime).ok()?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .ends_with(suffix.as_str())
        {
            return Some(entry.path());
        }
    }
    None
}

fn set_session_password(runtime: &Path, session_name: &str, password: &[u8]) {
    let socket_path = session_socket_path(runtime, session_name)
        .unwrap_or_else(|| panic!("missing socket for session {session_name}"));
    let mut stream = UnixStream::connect(&socket_path)
        .unwrap_or_else(|error| panic!("connect {}: {error}", socket_path.display()));
    screen_protocol::Message::Hello
        .write_to(&mut stream)
        .expect("write hello");
    match screen_protocol::Message::read_from(&mut stream).expect("read hello ack") {
        screen_protocol::Message::HelloAck => {}
        message => panic!("unexpected hello response: {message:?}"),
    }

    let mut command = b"password ".to_vec();
    command.extend_from_slice(password);
    screen_protocol::Message::Command(command)
        .write_to(&mut stream)
        .expect("write password command");
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
