use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screen_testkit::{PtySize, PtyTestProcess};

#[test]
fn prefix_create_prev_next_detach_compares_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    let ref_dir = TempDir::new("prefix-ref");
    let cand_dir = TempDir::new("prefix-cand");
    if !ready(&cand_dir) || !ready(&ref_dir) {
        return;
    }

    let ref_result = run_prefix_case(&reference, ref_dir.path(), "prefref");
    let cand_result = run_prefix_case(&candidate, cand_dir.path(), "prefcand");

    if ref_result != cand_result {
        eprintln!("=== prefix differential report ===");
        eprintln!("reference: {ref_result:#?}");
        eprintln!("candidate: {cand_result:#?}");
    }

    assert!(ref_result.completed, "reference prefix case failed");
    assert!(cand_result.completed, "candidate prefix case failed");
    assert_eq!(cand_result, ref_result);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrefixCaseResult {
    completed: bool,
    probes: Vec<NumberProbe>,
    detach_status: i32,
    diagnostics: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NumberProbe {
    label: &'static str,
    status: i32,
    selected: Option<u32>,
}

fn run_prefix_case(exe: &Path, runtime: &Path, tag: &str) -> PrefixCaseResult {
    let session_name = format!("{tag}-prefix");
    let mut result = PrefixCaseResult {
        completed: false,
        probes: Vec::new(),
        detach_status: 1,
        diagnostics: String::new(),
    };

    let script = "printf PREFIX_READY; while true; do sleep 1; done";
    let args = [
        OsString::from("-S"),
        OsString::from(&session_name),
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(script),
    ];
    let envs = vec![
        (OsString::from("SCREENDIR"), runtime.as_os_str().to_owned()),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];

    let mut process =
        match PtyTestProcess::spawn_with_env(exe, args.iter(), envs, PtySize::new(80, 24)) {
            Ok(process) => process,
            Err(error) => {
                result.diagnostics = format!("spawn attached session: {error}");
                return result;
            }
        };

    if let Err(error) = process.read_until(b"PREFIX_READY", Duration::from_secs(5)) {
        result.diagnostics = format!("read readiness marker: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if !wait_until_session_present(exe, runtime, &session_name) {
        result.diagnostics = "session did not become visible".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Some(probe) = query_number_until(exe, runtime, &session_name, "initial", Some(0)) {
        result.probes.push(probe);
    } else {
        result.diagnostics = "initial selected window was not 0".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Err(error) = process.send(b"\x01c") {
        result.diagnostics = format!("send C-a c: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if let Some(probe) = query_number_until(exe, runtime, &session_name, "after_create", Some(1)) {
        result.probes.push(probe);
    } else {
        result.diagnostics = "C-a c did not select window 1".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Err(error) = process.send(b"\x01p") {
        result.diagnostics = format!("send C-a p: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if let Some(probe) = query_number_until(exe, runtime, &session_name, "after_prev", Some(0)) {
        result.probes.push(probe);
    } else {
        result.diagnostics = "C-a p did not select window 0".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Err(error) = process.send(b"\x01n") {
        result.diagnostics = format!("send C-a n: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if let Some(probe) = query_number_until(exe, runtime, &session_name, "after_next", Some(1)) {
        result.probes.push(probe);
    } else {
        result.diagnostics = "C-a n did not select window 1".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Err(error) = process.send(b"\x011") {
        result.diagnostics = format!("send C-a 1: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if let Some(probe) = query_number_until(exe, runtime, &session_name, "after_digit_1", Some(1)) {
        result.probes.push(probe);
    } else {
        result.diagnostics = "C-a 1 did not select window 1".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Err(error) = process.send(b"\x01 ") {
        result.diagnostics = format!("send C-a space: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    if let Some(probe) = query_number_until(exe, runtime, &session_name, "after_space", Some(0)) {
        result.probes.push(probe);
    } else {
        result.diagnostics = "C-a space did not advance to window 0".to_owned();
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }

    if let Err(error) = process.send(b"\x01d") {
        result.diagnostics = format!("send C-a d: {error}");
        let _ = quit_session(exe, runtime, &session_name);
        return result;
    }
    match process.wait_or_kill(Duration::from_secs(5)) {
        Ok(status) => result.detach_status = status.code().unwrap_or(1),
        Err(error) => {
            result.diagnostics = format!("wait for detach: {error}");
            let _ = quit_session(exe, runtime, &session_name);
            return result;
        }
    }

    let _ = quit_session(exe, runtime, &session_name);
    result.completed = result.diagnostics.is_empty() && result.detach_status == 0;
    result
}

fn query_number_until(
    exe: &Path,
    runtime: &Path,
    session_name: &str,
    label: &'static str,
    expected: Option<u32>,
) -> Option<NumberProbe> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        match query_number(exe, runtime, session_name, label) {
            Ok(probe) => {
                if expected.is_none() || probe.selected == expected {
                    return Some(probe);
                }
                last = Some(probe);
            }
            Err(error) => {
                last = Some(NumberProbe {
                    label,
                    status: 1,
                    selected: parse_selected_number(error.as_bytes()),
                });
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    last
}

fn query_number(
    exe: &Path,
    runtime: &Path,
    session_name: &str,
    label: &'static str,
) -> Result<NumberProbe, String> {
    let args = [
        OsStr::new("-S"),
        OsStr::new(session_name),
        OsStr::new("-Q"),
        OsStr::new("number"),
    ];
    let output = run_screen(exe, runtime, &args, Duration::from_secs(5))?;
    Ok(NumberProbe {
        label,
        status: output.status,
        selected: parse_selected_number(&output.stdout),
    })
}

fn parse_selected_number(output: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(output);
    let digits: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[derive(Debug)]
struct CommandOutput {
    status: i32,
    stdout: Vec<u8>,
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
    let _stderr = fs::read(&stderr_path).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&stdout_path);
    let _ = fs::remove_file(&stderr_path);

    Ok(CommandOutput {
        status: status.code().unwrap_or(1),
        stdout,
    })
}

fn quit_session(exe: &Path, runtime: &Path, session_name: &str) -> Result<CommandOutput, String> {
    let args = [
        OsStr::new("-S"),
        OsStr::new(session_name),
        OsStr::new("-X"),
        OsStr::new("quit"),
    ];
    run_screen(exe, runtime, &args, Duration::from_secs(5))
}

fn wait_until_session_present(exe: &Path, runtime: &Path, session_name: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let args = [OsStr::new("-ls")];
        if let Ok(output) = run_screen(exe, runtime, &args, Duration::from_secs(5))
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
        eprintln!("skipping prefix differential test: Unix socket bind is not permitted");
        return false;
    }
    if !screen_testkit::pty_available() {
        eprintln!("skipping prefix differential test: PTY not available");
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
