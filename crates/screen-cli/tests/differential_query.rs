use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn q_core_commands_compare_with_gnu_screen() {
    let reference = env_ref();
    let candidate = env_cand();

    if !reference_supports_query(&reference) {
        eprintln!("skipping test: GNU Screen reference does not support -Q");
        return;
    }

    let ref_dir = TempDir::new("q-ref");
    let cand_dir = TempDir::new("q-cand");
    if !ready(&cand_dir) || !ready(&ref_dir) {
        return;
    }

    let ref_result = run_query_case(&reference, ref_dir.path(), "qref");
    let cand_result = run_query_case(&candidate, cand_dir.path(), "qcand");

    if ref_result != cand_result {
        eprintln!("=== -Q differential report ===");
        eprintln!("reference: {ref_result:#?}");
        eprintln!("candidate: {cand_result:#?}");
    }

    assert_eq!(cand_result, ref_result);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryCaseResult {
    completed: bool,
    probes: Vec<ProbeResult>,
    diagnostics: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeResult {
    command: &'static str,
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_query_case(exe: &Path, runtime: &Path, tag: &str) -> QueryCaseResult {
    let session_name = format!("{tag}-query");
    let title = b"query-title";
    let script = format!(
        "printf '\\033]2;{}\\007'; while true; do sleep 1; done",
        String::from_utf8_lossy(title)
    );
    let start_args = [
        OsStr::new("-dmS"),
        OsStr::new(&session_name),
        OsStr::new("/bin/sh"),
        OsStr::new("-c"),
        OsStr::new(&script),
    ];

    if let Err(error) = run_screen(exe, runtime, &start_args, Duration::from_secs(10)) {
        return QueryCaseResult {
            completed: false,
            probes: Vec::new(),
            diagnostics: format!("start failed: {error}"),
        };
    }

    if !wait_until_session_present(exe, runtime, &session_name) {
        return QueryCaseResult {
            completed: false,
            probes: Vec::new(),
            diagnostics: "session did not become visible".to_owned(),
        };
    }

    let mut probes = Vec::new();
    for command in [
        "windows",
        "number",
        "title",
        "sessionname",
        "stuff",
        "kill",
        "quit",
        "screen",
        "help",
        "license",
        "info",
        "lastmsg",
        "time",
        "version",
    ] {
        let args = [
            OsStr::new("-S"),
            OsStr::new(&session_name),
            OsStr::new("-Q"),
            OsStr::new(command),
        ];
        match run_screen(exe, runtime, &args, Duration::from_secs(10)) {
            Ok(output) => probes.push(ProbeResult {
                command,
                status: normalize_probe_status(command, output.status),
                stdout: normalize_probe_stdout(command, output.stdout, &session_name),
                stderr: normalize_probe_stderr(output.stderr),
            }),
            Err(error) => {
                let _ = quit_session(exe, runtime, &session_name);
                return QueryCaseResult {
                    completed: false,
                    probes,
                    diagnostics: format!("query {command} failed: {error}"),
                };
            }
        }
    }

    let _ = quit_session(exe, runtime, &session_name);

    QueryCaseResult {
        completed: true,
        probes,
        diagnostics: String::new(),
    }
}

fn normalize_probe_status(command: &str, status: i32) -> i32 {
    match command {
        // GNU Screen 4.9.1 and 5.0.2 differ on these query statuses; keep the
        // stream-routing coverage while avoiding a cross-version status claim.
        "lastmsg" | "version" => -1,
        _ => status,
    }
}

fn normalize_probe_stdout(command: &str, stdout: Vec<u8>, _session_name: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(&stdout).replace('\r', "");
    match command {
        "number" => {
            let selected: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            format!("<number:{selected}>\n").into_bytes()
        }
        "windows" | "title" | "info" | "lastmsg" | "time" | "version" => {
            // These commands contain version-specific formatting, terminal title,
            // clock, or message-line content. The differential assertion here is
            // status/stream routing plus command availability.
            let _ = text;
            format!("<{command}>\n").into_bytes()
        }
        "sessionname" | "stuff" | "kill" | "quit" | "screen" | "help" | "license" => {
            format!("<nonqueryable:{command}>\n").into_bytes()
        }
        _ => text.into_bytes(),
    }
}

fn normalize_probe_stderr(stderr: Vec<u8>) -> Vec<u8> {
    String::from_utf8_lossy(&stderr)
        .replace('\r', "")
        .into_bytes()
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

fn env_ref() -> PathBuf {
    std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"))
}

fn reference_supports_query(reference: &Path) -> bool {
    Command::new(reference)
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map(|output| {
            let mut combined = output.stdout;
            combined.extend_from_slice(&output.stderr);
            combined.windows(2).any(|window| window == b"-Q")
        })
        .unwrap_or(false)
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
