use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn multi_window_select_renumber_title_kill_compares_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    let ref_dir = TempDir::new("mw-ref");
    let cand_dir = TempDir::new("mw-cand");
    if !ready(&cand_dir) || !ready(&ref_dir) {
        return;
    }

    let ref_result = run_multi_window_case(&reference, ref_dir.path(), "mwref");
    let cand_result = run_multi_window_case(&candidate, cand_dir.path(), "mwcand");

    if !ref_result.completed || !cand_result.completed || ref_result != cand_result {
        eprintln!("=== multi-window differential report ===");
        eprintln!("reference: {ref_result:#?}");
        eprintln!("candidate: {cand_result:#?}");
    }

    assert!(ref_result.completed, "reference multi-window case failed");
    assert!(cand_result.completed, "candidate multi-window case failed");
    assert_eq!(cand_result, ref_result);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MultiWindowCaseResult {
    completed: bool,
    probes: Vec<WindowProbe>,
    diagnostics: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowProbe {
    label: &'static str,
    status: i32,
    selected: Option<u32>,
    title: Option<Vec<u8>>,
}

fn run_multi_window_case(exe: &Path, runtime: &Path, tag: &str) -> MultiWindowCaseResult {
    let session_name = format!("{tag}-mw");
    let mut result = MultiWindowCaseResult {
        completed: false,
        probes: Vec::new(),
        diagnostics: String::new(),
    };

    let zero_script = "printf '\\033]2;zero\\007'; while true; do sleep 1; done";
    if let Err(error) = run_screen(
        exe,
        runtime,
        &[
            OsStr::new("-dmS"),
            OsStr::new(&session_name),
            OsStr::new("/bin/sh"),
            OsStr::new("-c"),
            OsStr::new(zero_script),
        ],
        Duration::from_secs(10),
    ) {
        result.diagnostics = format!("start failed: {error}");
        return result;
    }

    if !wait_until_session_present(exe, runtime, &session_name) {
        result.diagnostics = "session did not become visible".to_owned();
        return result;
    }

    if !push_probe_until(
        exe,
        runtime,
        &session_name,
        &mut result,
        "initial",
        Some(0),
        None,
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    let one_script = "printf '\\033]2;one\\007'; while true; do sleep 1; done";
    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-X", "screen", "/bin/sh", "-c", one_script],
        &mut result,
        "create window 1",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-X", "select", "1"],
        &mut result,
        "select window 1",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if !push_probe_until(
        exe,
        runtime,
        &session_name,
        &mut result,
        "select_1",
        Some(1),
        None,
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-X", "number", "5"],
        &mut result,
        "renumber window 1 to 5",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if !push_probe_until(
        exe,
        runtime,
        &session_name,
        &mut result,
        "after_renumber",
        Some(5),
        None,
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-X", "select", "0"],
        &mut result,
        "select window 0 for title",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-p", "0", "-X", "title", "zero-renamed"],
        &mut result,
        "rename window 0",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-X", "select", "0"],
        &mut result,
        "reselect window 0 after title",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if !remote_ok(
        exe,
        runtime,
        &session_name,
        &["-X", "kill", "5"],
        &mut result,
        "kill window 5",
    ) {
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    let _ = quit_session(exe, runtime, &session_name);
    result.completed = result.diagnostics.is_empty();
    result
}

fn remote_ok(
    exe: &Path,
    runtime: &Path,
    session_name: &str,
    command: &[&str],
    result: &mut MultiWindowCaseResult,
    label: &str,
) -> bool {
    let mut args = vec![OsStr::new("-S"), OsStr::new(session_name)];
    args.extend(command.iter().map(OsStr::new));
    match run_screen(exe, runtime, &args, Duration::from_secs(10)) {
        Ok(output) if output.status == 0 => true,
        Ok(output) => {
            result.diagnostics = format!(
                "{label}: status={} stdout={:?} stderr={:?}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            false
        }
        Err(error) => {
            result.diagnostics = format!("{label}: {error}");
            false
        }
    }
}

fn push_probe_until(
    exe: &Path,
    runtime: &Path,
    session_name: &str,
    result: &mut MultiWindowCaseResult,
    label: &'static str,
    selected: Option<u32>,
    title: Option<&[u8]>,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        match probe_window(exe, runtime, session_name, label) {
            Ok(probe)
                if (selected.is_none() || probe.selected == selected)
                    && (title.is_none() || probe.title.as_deref() == title) =>
            {
                let mut probe = probe;
                if title.is_none() {
                    probe.title = None;
                }
                result.probes.push(probe);
                return true;
            }
            Ok(probe) => last = Some(probe),
            Err(error) => result.diagnostics = format!("{label}: {error}"),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    result.diagnostics = format!(
        "{label}: expected selected={selected:?} title={:?}, last={last:?}",
        title.map(String::from_utf8_lossy)
    );
    false
}

fn probe_window(
    exe: &Path,
    runtime: &Path,
    session_name: &str,
    label: &'static str,
) -> Result<WindowProbe, String> {
    let number = query(exe, runtime, session_name, "number")?;
    let title = query(exe, runtime, session_name, "title")?;
    Ok(WindowProbe {
        label,
        status: number.status,
        selected: parse_selected_number(&number.stdout),
        title: if title.status == 0 {
            Some(strip_cr(title.stdout))
        } else {
            None
        },
    })
}

fn query(
    exe: &Path,
    runtime: &Path,
    session_name: &str,
    command: &str,
) -> Result<CommandOutput, String> {
    run_screen(
        exe,
        runtime,
        &[
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-Q"),
            OsStr::new(command),
        ],
        Duration::from_secs(5),
    )
}

fn parse_selected_number(output: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(output);
    let digits: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn strip_cr(bytes: Vec<u8>) -> Vec<u8> {
    bytes.into_iter().filter(|b| *b != b'\r').collect()
}

#[derive(Debug)]
struct CommandOutput {
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_screen(
    executable: &Path,
    runtime: &Path,
    args: &[&OsStr],
    timeout: Duration,
) -> Result<CommandOutput, String> {
    fs::create_dir_all(runtime).map_err(|e| e.to_string())?;
    fs::set_permissions(runtime, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;

    let stdout_path = runtime.join(format!("stdout-{}", nanos()));
    let stderr_path = runtime.join(format!("stderr-{}", nanos()));
    let stdout_file = fs::File::create(&stdout_path).map_err(|e| e.to_string())?;
    let stderr_file = fs::File::create(&stderr_path).map_err(|e| e.to_string())?;

    let mut child = Command::new(executable)
        .args(args)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            stdout_file.try_clone().map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            stderr_file.try_clone().map_err(|e| e.to_string())?,
        ))
        .spawn()
        .map_err(|e| e.to_string())?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
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

    Ok(CommandOutput {
        status: status.code().unwrap_or(1),
        stdout,
        stderr,
    })
}

fn quit_session(exe: &Path, runtime: &Path, session_name: &str) -> Result<CommandOutput, String> {
    run_screen(
        exe,
        runtime,
        &[
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-X"),
            OsStr::new("quit"),
        ],
        Duration::from_secs(5),
    )
}

fn wait_until_session_present(exe: &Path, runtime: &Path, session_name: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(output) = run_screen(exe, runtime, &[OsStr::new("-ls")], Duration::from_secs(5))
            && String::from_utf8_lossy(&output.stdout).contains(session_name)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

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
        eprintln!("skipping multi-window differential test: Unix socket bind is not permitted");
        return false;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping multi-window differential test: PTY not available");
        return false;
    }
    true
}

fn unix_socket_bind_allowed(path: &Path) -> bool {
    let test_sock = path.join("_bind_test");
    match std::os::unix::net::UnixListener::bind(&test_sock) {
        Ok(listener) => {
            drop(listener);
            let _ = fs::remove_file(&test_sock);
            true
        }
        Err(_) => false,
    }
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("screen-rs-test-{prefix}-{}", nanos()));
        let _ = fs::create_dir_all(&dir);
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        Self { path: dir }
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
