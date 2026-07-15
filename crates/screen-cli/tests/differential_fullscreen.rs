//! Differential full-screen application tests against GNU Screen reference.
//!
//! These tests verify that screen-rs correctly handles terminal apps that use
//! alternate screens, cursor positioning, and other advanced TUI features.
//!
//! All full-screen test apps are self-contained (pure `sh` scripts using `tput` and
//! `printf`), so no external programs (vim, top, less) are required.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use screen_testkit::{PtySize, PtyTestProcess};

/// Simple runner: starts screen-rs attached with the given command, reads until a
/// marker appears in the PTY output, and returns everything read.
fn run_fullscreen_until(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
    shell_cmd: &str,
    marker: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let screenrc_path = runtime.join("fs-screenrc");
    fs::write(&screenrc_path, "startup_message off\n").map_err(|e| e.to_string())?;
    let envs = [
        (OsString::from("SCREENDIR"), runtime.as_os_str().to_owned()),
        (
            OsString::from("SCREENRC"),
            screenrc_path.as_os_str().to_owned(),
        ),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut process = PtyTestProcess::spawn_with_env(
        executable,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new(shell_cmd),
        ],
        envs,
        PtySize::new(80, 24),
    )
    .map_err(|e| format!("spawn: {e}"))?;

    process
        .read_until(marker, timeout)
        .map_err(|e| format!("read_until({marker:?}): {e}"))
}

// ── Test: alternate screen text capture ────────────────────────────────────

#[test]
fn alternate_screen_text_is_visible() {
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let runtime = TempDir::new("alt-screen-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    // `tput smcup` enters alternate screen.  `printf` writes visible text,
    // then `sleep` keeps the child alive so we have time to read.
    let script = "tput smcup; printf '\\x1b[1;1HALT_MARKER'; sleep 1; tput rmcup";
    let output = match run_fullscreen_until(
        &candidate,
        runtime.path(),
        "alt-text",
        script,
        b"ALT_MARKER",
        Duration::from_secs(5),
    ) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("alternate_screen_text_is_visible: {e}");
            panic!("{e}");
        }
    };

    assert!(
        contains(&output, b"ALT_MARKER"),
        "alternate-screen text not visible in output"
    );
    // The alternate-screen enter sequence must be present.
    assert!(
        contains(&output, b"\x1b[?1049h"),
        "missing ESC[?1049h (smcup)"
    );
}

// ── Test: alternate screen with GNU Screen comparison ──────────────────────

#[test]
fn alternate_screen_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let ref_runtime = TempDir::new("alt-cmp-ref");
    let cand_runtime = TempDir::new("alt-cmp-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    // A script that enters alternate screen, writes markers at two positions,
    // then exits normally.
    let script = "tput smcup; printf '\\x1b[1;1HALT_TOP'; printf '\\x1b[10;1HALT_BOT'; sleep 1; tput rmcup; echo 'ALT_DONE'";

    let ref_out = run_fullscreen_until(
        &reference,
        ref_runtime.path(),
        "alt-gnu",
        script,
        b"ALT_DONE",
        Duration::from_secs(8),
    );

    let cand_out = run_fullscreen_until(
        &candidate,
        cand_runtime.path(),
        "alt-cand",
        script,
        b"ALT_DONE",
        Duration::from_secs(8),
    );

    match (&ref_out, &cand_out) {
        (Ok(ref_o), Ok(cand_o)) => {
            let ref_has_top = contains(ref_o, b"ALT_TOP");
            let ref_has_bot = contains(ref_o, b"ALT_BOT");
            let cand_has_top = contains(cand_o, b"ALT_TOP");
            let cand_has_bot = contains(cand_o, b"ALT_BOT");

            if ref_has_top != cand_has_top || ref_has_bot != cand_has_bot {
                eprintln!("alternate_screen comparison:");
                eprintln!("  reference top={ref_has_top} bot={ref_has_bot}");
                eprintln!("  candidate top={cand_has_top} bot={cand_has_bot}");
            }
            // If reference works, candidate must match.
            if ref_has_top {
                assert!(cand_has_top, "candidate missing ALT_TOP text");
            }
            if ref_has_bot {
                assert!(cand_has_bot, "candidate missing ALT_BOT text");
            }
            // At minimum, candidate must complete.
            assert!(contains(cand_o, b"ALT_DONE"), "candidate did not complete");
        }
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("alternate_screen_compares: ref={ref_out:?} cand={cand_out:?}");
            assert!(cand_out.is_ok(), "candidate failed: {e}");
        }
    }
}

// ── Test: cursor explicit-position text ────────────────────────────────────

#[test]
fn cursor_positioned_text_is_visible() {
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let runtime = TempDir::new("cursor-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    // Move cursor to (5,10), print marker.  This tests that screen-rs passes
    // through cursor-positioning escapes correctly.
    let script = "printf '\\x1b[5;10HCURSOR_MARK'; sleep 1; echo 'CURSOR_DONE'";

    let output = run_fullscreen_until(
        &candidate,
        runtime.path(),
        "cursor",
        script,
        b"CURSOR_DONE",
        Duration::from_secs(5),
    )
    .expect("cursor test should complete");

    assert!(contains(&output, b"CURSOR_MARK"), "cursor text not visible");
    // The cursor movement escape must be present.
    assert!(
        contains(&output, b"\x1b[5;10H"),
        "missing cursor-position escape"
    );
}

// ── Test: resize while alternate screen is active ──────────────────────────

#[test]
fn resize_during_alternate_screen_does_not_crash() {
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let runtime = TempDir::new("resize-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    let script = "tput smcup; printf '\\x1b[1;1HRESIZE_BEFORE'; sleep 1; printf '\\x1b[2;1HRESIZE_AFTER'; sleep 1; tput rmcup; echo 'RESIZE_DONE'";

    let screenrc_path = runtime.path().join("resize-screenrc");
    fs::write(&screenrc_path, "startup_message off\n").unwrap();
    let envs = [
        (
            OsString::from("SCREENDIR"),
            runtime.path().as_os_str().to_owned(),
        ),
        (
            OsString::from("SCREENRC"),
            screenrc_path.as_os_str().to_owned(),
        ),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];

    let mut process = PtyTestProcess::spawn_with_env(
        &candidate,
        [
            OsStr::new("-S"),
            OsStr::new("resize-fs"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new(script),
        ],
        envs,
        PtySize::new(80, 24),
    )
    .expect("resize test spawn");

    // Read until we know the child is in the alternate screen.
    let _before = process
        .read_until(b"RESIZE_BEFORE", Duration::from_secs(5))
        .expect("before-resize marker");

    // Resize the PTY *while* the child is in alternate screen mode.
    process
        .resize(PtySize::new(100, 30))
        .expect("resize should succeed");

    // Read the "after resize" marker.
    let after = process
        .read_until(b"RESIZE_DONE", Duration::from_secs(5))
        .expect("after-resize marker");

    assert!(
        contains(&after, b"RESIZE_AFTER"),
        "child still alive after resize in alt-screen"
    );
    assert!(
        contains(&after, b"RESIZE_DONE"),
        "child completed after resize"
    );
}

// ── Test: multi-window with full-screen content ────────────────────────────

#[test]
fn multi_window_fullscreen_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let ref_runtime = TempDir::new("mwfs-ref");
    let cand_runtime = TempDir::new("mwfs-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    // Run the same multi-window scenario against both implementations.
    let ref_ok = run_multi_window_fullscreen_test(&reference, ref_runtime.path(), "mwfs-ref");
    let cand_ok = run_multi_window_fullscreen_test(&candidate, cand_runtime.path(), "mwfs-cand");

    if !cand_ok {
        eprintln!("multi_window_fullscreen: candidate failed");
    }
    if ref_ok && !cand_ok {
        eprintln!("multi_window_fullscreen: reference passed but candidate failed");
    }

    assert!(cand_ok, "candidate multi-window fullscreen test failed");

    // Reference is best-effort — only compare if it also succeeded.
    if ref_ok {
        assert!(cand_ok, "candidate failed while reference passed");
    }
}

/// Helper: start a detached session, create two windows running simple
/// full-screen scripts, and verify the session stays healthy.
fn run_multi_window_fullscreen_test(executable: &Path, runtime: &Path, session_name: &str) -> bool {
    // Start a detached session with a long-running loop (same pattern as
    // multi_window_create_and_kill test in differential_session.rs).
    let start = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("while :; do sleep 1; done"),
        ],
        Duration::from_secs(5),
    );
    let Ok(start_status) = start else {
        eprintln!("mwfs: start failed to spawn");
        return false;
    };
    if !start_status.success() {
        eprintln!("mwfs: start exited with {start_status}");
        return false;
    }

    // Create a second window with a full-screen-style command.
    let create = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("screen"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("tput smcup; printf '\\x1b[1;1HW2_READY'; while :; do sleep 1; done"),
        ],
        Duration::from_secs(5),
    );
    let Ok(create_status) = create else {
        eprintln!("mwfs: create window failed to spawn");
        let _ = quit_session(executable, runtime, session_name);
        return false;
    };
    if !create_status.success() {
        eprintln!("mwfs: create window exited with {create_status}");
        let _ = quit_session(executable, runtime, session_name);
        return false;
    }

    // Verify session still shows up in listing.
    let list = run_screen_output(
        executable,
        runtime,
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    );
    let Ok(list_out) = list else {
        eprintln!("mwfs: list failed");
        let _ = quit_session(executable, runtime, session_name);
        return false;
    };

    let survived = output_lists_session(&list_out, session_name);
    if !survived {
        eprintln!("mwfs: session not in listing after create");
    }

    // Clean up.
    let _ = quit_session(executable, runtime, session_name);
    let cleanup = wait_until_no_session(executable, runtime, session_name);
    if !cleanup {
        eprintln!("mwfs: session did not clean up");
    }

    survived && cleanup
}

// ── Test: SGR attributes (bold, underline, colors) passed through ──────────

#[test]
fn sgr_attributes_preserved_for_client() {
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let runtime = TempDir::new("sgr-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    // Emit bold text using SGR codes.  screen-rs should pass these through
    // so the client terminal can render them.
    // note: sleep 1 after output to avoid macOS PTY data-loss race (child
    // must stay alive until the daemon has a chance to read PTY output).
    let script = "printf '\\x1b[1mBOLD_TEXT\\x1b[0m'; sleep 1; echo 'SGR_DONE'";

    let output = run_fullscreen_until(
        &candidate,
        runtime.path(),
        "sgr-pass",
        script,
        b"SGR_DONE",
        Duration::from_secs(5),
    )
    .expect("SGR test should complete");

    // The SGR sequences must be forwarded to the client.
    assert!(
        contains(&output, b"\x1b[1m"),
        "missing bold SGR sequence in output"
    );
    assert!(contains(&output, b"BOLD_TEXT"), "missing bold text content");
    assert!(contains(&output, b"\x1b[0m"), "missing SGR reset sequence");
}

// ── Test: terminal bell is forwarded ────────────────────────────────────────

#[test]
fn terminal_bell_is_forwarded_to_client() {
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let runtime = TempDir::new("bell-cand");
    if !screen_testkit::pty_available() {
        eprintln!("skipping: PTY allocation not available");
        return;
    }

    // Emit BEL character and verify it's passed through.
    let script = "printf '\\x07BELL_OUT'; sleep 1; echo 'BELL_DONE'";

    let output = run_fullscreen_until(
        &candidate,
        runtime.path(),
        "bell",
        script,
        b"BELL_DONE",
        Duration::from_secs(5),
    )
    .expect("bell test should complete");

    assert!(contains(&output, &[0x07]), "missing BEL character");
    assert!(contains(&output, b"BELL_OUT"), "text after BEL");
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn run_screen_null(
    executable: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    timeout: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    use std::process::{Command, Stdio};
    let home = ensure_test_home(runtime);
    let stderr_path = runtime.join(format!("fs-stderr-{}", unique_nanos()));
    let stderr_file = fs::File::create(&stderr_path)?;
    let mut child = Command::new(executable)
        .args(args)
        .env("HOME", &home)
        .env("ZDOTDIR", &home)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file.try_clone()?))
        .spawn()?;

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "run_screen_null timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Capture and display stderr for debugging failures
    drop(stderr_file);
    if let Ok(stderr_bytes) = fs::read(&stderr_path)
        && !stderr_bytes.is_empty()
    {
        eprintln!(
            "run_screen_null stderr:\n{}",
            String::from_utf8_lossy(&stderr_bytes)
        );
    }
    let _ = fs::remove_file(&stderr_path);
    Ok(status)
}

#[allow(dead_code)]
struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_screen_output(
    executable: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    timeout: Duration,
) -> std::io::Result<CommandOutput> {
    use std::process::{Command, Stdio};

    let home = ensure_test_home(runtime);
    let stdout_path = runtime.join(format!("fs-stdout-{}", unique_nanos()));
    let stderr_path = runtime.join(format!("fs-stderr-{}", unique_nanos()));
    let stdout_file = fs::File::create(&stdout_path)?;
    let stderr_file = fs::File::create(&stderr_path)?;

    let mut child = Command::new(executable)
        .args(args)
        .env("HOME", &home)
        .env("ZDOTDIR", &home)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file.try_clone()?))
        .stderr(Stdio::from(stderr_file.try_clone()?))
        .spawn()?;

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "run_screen timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    drop(stdout_file);
    drop(stderr_file);
    let stdout = fs::read(&stdout_path)?;
    let stderr = fs::read(&stderr_path)?;
    let _ = fs::remove_file(stdout_path);
    let _ = fs::remove_file(stderr_path);

    Ok(CommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn quit_session(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> std::io::Result<CommandOutput> {
    run_screen_output(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(5),
    )
}

fn wait_until_no_session(executable: &Path, runtime: &Path, session_name: &str) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(list) = run_screen_output(
            executable,
            runtime,
            [OsStr::new("-ls")],
            Duration::from_secs(2),
        ) && !output_lists_session(&list, session_name)
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            let _ = quit_session(executable, runtime, session_name);
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn output_lists_session(output: &CommandOutput, session_name: &str) -> bool {
    let dotted = format!(".{session_name}");
    let dotted_bytes = dotted.as_bytes();
    output
        .stdout
        .split(|b| *b == b'\n')
        .any(|line| contains(trim_ascii_start(line), dotted_bytes))
        || output
            .stderr
            .split(|b| *b == b'\n')
            .any(|line| contains(trim_ascii_start(line), dotted_bytes))
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let first = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[first..]
}

fn ensure_test_home(runtime: &Path) -> PathBuf {
    let home = runtime.join("home");
    let _ = fs::create_dir_all(&home);
    home
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

fn unique_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

// ── TempDir (copied from differential_session.rs) ─────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};

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
        let short = if name.len() > 8 { &name[..8] } else { name };
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("fs-{short}-{}-{seq}", std::process::id()));
        fs::create_dir(&path).unwrap_or_else(|e| panic!("failed to create temp dir {path:?}: {e}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
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
