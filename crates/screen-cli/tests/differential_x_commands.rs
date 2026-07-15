use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Test screen-rs vs GNU Screen for `-X screen <program>` (create window with program).
#[test]
fn x_screen_create_window_compares_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    let ref_dir = TempDir::new("x-scr-ref");
    let cand_dir = TempDir::new("x-scr-cand");
    if !ready(&cand_dir) {
        return;
    }

    let ref_result = run_x_screen_case(&reference, ref_dir.path(), "xscr");
    let cand_result = run_x_screen_case(&candidate, cand_dir.path(), "xscr");

    report("x_screen_create_window", &ref_result, &cand_result);
    assert!(ref_result.completed, "reference failed");
    assert!(cand_result.completed, "candidate failed");
    assert_eq!(cand_result.output, ref_result.output);
}

/// Test screen-rs vs GNU Screen for `-X clear`.
#[test]
fn x_clear_compares_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    let ref_dir = TempDir::new("x-clr-ref");
    let cand_dir = TempDir::new("x-clr-cand");
    if !ready(&cand_dir) {
        return;
    }

    let ref_result = run_x_clear_case(&reference, ref_dir.path(), "xclr");
    let cand_result = run_x_clear_case(&candidate, cand_dir.path(), "xclr");

    report("x_clear", &ref_result, &cand_result);
    assert!(ref_result.completed, "reference failed");
    assert!(cand_result.completed, "candidate failed");
    assert_eq!(cand_result.output, ref_result.output);
}

/// Test screen-rs vs GNU Screen for `-X fit` in a split layout.
#[test]
fn x_fit_compares_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    let ref_dir = TempDir::new("x-fit-ref");
    let cand_dir = TempDir::new("x-fit-cand");
    if !ready(&cand_dir) {
        return;
    }

    let ref_result = run_x_fit_case(&reference, ref_dir.path(), "xfit");
    let cand_result = run_x_fit_case(&candidate, cand_dir.path(), "xfit");

    report("x_fit", &ref_result, &cand_result);
    assert!(ref_result.completed, "reference failed");
    assert!(cand_result.completed, "candidate failed");
    assert_eq!(cand_result.output, ref_result.output);
}

/// Test screen-rs vs GNU Screen for `-X reset`.
#[test]
fn x_reset_compares_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    let ref_dir = TempDir::new("x-rst-ref");
    let cand_dir = TempDir::new("x-rst-cand");
    if !ready(&cand_dir) {
        return;
    }

    let ref_result = run_x_reset_case(&reference, ref_dir.path(), "xrst");
    let cand_result = run_x_reset_case(&candidate, cand_dir.path(), "xrst");

    report("x_reset", &ref_result, &cand_result);
    assert!(ref_result.completed, "reference failed");
    assert!(cand_result.completed, "candidate failed");
    assert_eq!(cand_result.output, ref_result.output);
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn env_ref() -> PathBuf {
    std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"))
}

fn env_cand() -> PathBuf {
    std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")))
}

fn ready(dir: &TempDir) -> bool {
    if !unix_socket_bind_allowed(dir.path()) {
        eprintln!("skipping test: Unix socket bind is not permitted");
        return false;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping test: PTY not available");
        return false;
    }
    true
}

fn unix_socket_bind_allowed(path: &Path) -> bool {
    // Check if socket binding is functional in this environment
    let test_sock = path.join("_bind_test");
    match std::os::unix::net::UnixListener::bind(&test_sock) {
        Ok(l) => {
            drop(l);
            let _ = std::fs::remove_file(&test_sock);
            true
        }
        Err(_) => false,
    }
}

fn report(name: &str, ref_r: &TestCaseResult, cand_r: &TestCaseResult) {
    if ref_r.output != cand_r.output || !ref_r.completed || !cand_r.completed {
        eprintln!("=== {name} differential report ===");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&ref_r.output)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&cand_r.output)
        );
        eprintln!("reference diag: {:?}", ref_r.diagnostics);
        eprintln!("candidate diag: {:?}", cand_r.diagnostics);
    }
}

struct TestCaseResult {
    completed: bool,
    output: Vec<u8>,
    diagnostics: String,
}

fn run_screen_output(
    executable: &Path,
    runtime: &Path,
    args: &[&OsStr],
    timeout: Duration,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let stdout_path = runtime.join(format!("so-stdout-{}", nanos()));
    let stderr_path = runtime.join(format!("so-stderr-{}", nanos()));
    let stdout_file = fs::File::create(&stdout_path).map_err(|e| e.to_string())?;
    let stderr_file = fs::File::create(&stderr_path).map_err(|e| e.to_string())?;

    let mut child = Command::new(executable)
        .args(args)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(
            stdout_file.try_clone().map_err(|e| e.to_string())?,
        ))
        .stderr(std::process::Stdio::from(
            stderr_file.try_clone().map_err(|e| e.to_string())?,
        ))
        .spawn()
        .map_err(|e| e.to_string())?;

    let deadline = std::time::Instant::now() + timeout;
    let _status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    drop(stdout_file);
    drop(stderr_file);
    let stdout = fs::read(&stdout_path).map_err(|e| e.to_string())?;
    let stderr = fs::read(&stderr_path).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&stdout_path);
    let _ = fs::remove_file(&stderr_path);

    Ok((stdout, stderr))
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

// ── Test case runners ──

/// -X screen /bin/echo hello
fn run_x_screen_case(exe: &Path, runtime: &Path, tag: &str) -> TestCaseResult {
    let _ = fs::create_dir_all(runtime);

    // 1. Start detached session
    let sess_name = format!("{tag}-sess");
    let start_args = &[
        OsStr::new("-dmS"),
        OsStr::new(&sess_name),
        OsStr::new("/bin/cat"),
    ];
    if let Err(e) = run_screen_output(exe, runtime, start_args, Duration::from_secs(10)) {
        return TestCaseResult {
            completed: false,
            output: Vec::new(),
            diagnostics: format!("start session failed: {e}"),
        };
    }
    std::thread::sleep(Duration::from_millis(300));

    // 2. Create a new window with /bin/echo hello
    let create_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-X"),
        OsStr::new("screen"),
        OsStr::new("/bin/echo"),
        OsStr::new("hello"),
    ];
    if let Err(e) = run_screen_output(exe, runtime, create_args, Duration::from_secs(10)) {
        return TestCaseResult {
            completed: false,
            output: Vec::new(),
            diagnostics: format!("create window failed: {e}"),
        };
    }
    std::thread::sleep(Duration::from_millis(300));

    // 3. Select window 1 (the new echo window) and stuff a newline to see output
    let select_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-p"),
        OsStr::new("1"),
        OsStr::new("-X"),
        OsStr::new("hardcopy"),
        OsStr::new("-h"),
    ];
    let (stdout, stderr) =
        match run_screen_output(exe, runtime, select_args, Duration::from_secs(10)) {
            Ok(r) => r,
            Err(e) => {
                return TestCaseResult {
                    completed: false,
                    output: Vec::new(),
                    diagnostics: format!("select failed: {e}"),
                };
            }
        };

    // 4. Quit
    let _ = run_screen_output(
        exe,
        runtime,
        &[
            OsStr::new("-S"),
            OsStr::new(&sess_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(5),
    );

    TestCaseResult {
        completed: true,
        output: stdout,
        diagnostics: String::from_utf8_lossy(&stderr).to_string(),
    }
}

/// -X clear
fn run_x_clear_case(exe: &Path, runtime: &Path, tag: &str) -> TestCaseResult {
    let _ = fs::create_dir_all(runtime);

    let sess_name = format!("{tag}-sess");
    let start_args = &[
        OsStr::new("-dmS"),
        OsStr::new(&sess_name),
        OsStr::new("/bin/cat"),
    ];
    if let Err(e) = run_screen_output(exe, runtime, start_args, Duration::from_secs(10)) {
        return TestCaseResult {
            completed: false,
            output: Vec::new(),
            diagnostics: format!("start failed: {e}"),
        };
    }
    std::thread::sleep(Duration::from_millis(300));

    // Write some text, then clear, then hardcopy
    let stuff_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-X"),
        OsStr::new("stuff"),
        OsStr::new("hello world\r"),
    ];
    let _ = run_screen_output(exe, runtime, stuff_args, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(200));

    let clear_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-X"),
        OsStr::new("clear"),
    ];
    let _ = run_screen_output(exe, runtime, clear_args, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(200));

    let hc_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-p"),
        OsStr::new("0"),
        OsStr::new("-X"),
        OsStr::new("hardcopy"),
        OsStr::new("-h"),
    ];
    let (stdout, stderr) = match run_screen_output(exe, runtime, hc_args, Duration::from_secs(10)) {
        Ok(r) => r,
        Err(e) => {
            return TestCaseResult {
                completed: false,
                output: Vec::new(),
                diagnostics: format!("hardcopy failed: {e}"),
            };
        }
    };

    let _ = run_screen_output(
        exe,
        runtime,
        &[
            OsStr::new("-S"),
            OsStr::new(&sess_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(5),
    );

    TestCaseResult {
        completed: true,
        output: stdout,
        diagnostics: String::from_utf8_lossy(&stderr).to_string(),
    }
}

/// -X fit
fn run_x_fit_case(exe: &Path, runtime: &Path, tag: &str) -> TestCaseResult {
    let _ = fs::create_dir_all(runtime);

    let sess_name = format!("{tag}-sess");
    let start_args = &[
        OsStr::new("-dmS"),
        OsStr::new(&sess_name),
        OsStr::new("/bin/cat"),
    ];
    if let Err(e) = run_screen_output(exe, runtime, start_args, Duration::from_secs(10)) {
        return TestCaseResult {
            completed: false,
            output: Vec::new(),
            diagnostics: format!("start failed: {e}"),
        };
    }
    std::thread::sleep(Duration::from_millis(300));

    // Split and then fit
    let split_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-X"),
        OsStr::new("split"),
    ];
    let _ = run_screen_output(exe, runtime, split_args, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(200));

    let fit_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-X"),
        OsStr::new("fit"),
    ];
    let _ = run_screen_output(exe, runtime, fit_args, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(200));

    let hc_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-p"),
        OsStr::new("0"),
        OsStr::new("-X"),
        OsStr::new("hardcopy"),
        OsStr::new("-h"),
    ];
    let (stdout, stderr) = match run_screen_output(exe, runtime, hc_args, Duration::from_secs(10)) {
        Ok(r) => r,
        Err(e) => {
            return TestCaseResult {
                completed: false,
                output: Vec::new(),
                diagnostics: format!("hardcopy failed: {e}"),
            };
        }
    };

    let _ = run_screen_output(
        exe,
        runtime,
        &[
            OsStr::new("-S"),
            OsStr::new(&sess_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(5),
    );

    TestCaseResult {
        completed: true,
        output: stdout,
        diagnostics: String::from_utf8_lossy(&stderr).to_string(),
    }
}

/// -X reset
fn run_x_reset_case(exe: &Path, runtime: &Path, tag: &str) -> TestCaseResult {
    let _ = fs::create_dir_all(runtime);

    let sess_name = format!("{tag}-sess");
    let start_args = &[
        OsStr::new("-dmS"),
        OsStr::new(&sess_name),
        OsStr::new("/bin/cat"),
    ];
    if let Err(e) = run_screen_output(exe, runtime, start_args, Duration::from_secs(10)) {
        return TestCaseResult {
            completed: false,
            output: Vec::new(),
            diagnostics: format!("start failed: {e}"),
        };
    }
    std::thread::sleep(Duration::from_millis(300));

    let reset_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-X"),
        OsStr::new("reset"),
    ];
    let _ = run_screen_output(exe, runtime, reset_args, Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(200));

    let hc_args = &[
        OsStr::new("-S"),
        OsStr::new(&sess_name),
        OsStr::new("-p"),
        OsStr::new("0"),
        OsStr::new("-X"),
        OsStr::new("hardcopy"),
        OsStr::new("-h"),
    ];
    let (stdout, stderr) = match run_screen_output(exe, runtime, hc_args, Duration::from_secs(10)) {
        Ok(r) => r,
        Err(e) => {
            return TestCaseResult {
                completed: false,
                output: Vec::new(),
                diagnostics: format!("hardcopy failed: {e}"),
            };
        }
    };

    let _ = run_screen_output(
        exe,
        runtime,
        &[
            OsStr::new("-S"),
            OsStr::new(&sess_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(5),
    );

    TestCaseResult {
        completed: true,
        output: stdout,
        diagnostics: String::from_utf8_lossy(&stderr).to_string(),
    }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("screen-rs-test-{}-{}", prefix, nanos()));
        let _ = fs::create_dir_all(&dir);
        TempDir { path: dir }
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
