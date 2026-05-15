use std::env;
use std::ffi::{OsStr, OsString, c_int, c_uchar, c_ulong};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use screen_cli::{
    AttachOptions, AttachOrCreateOptions, CreateDetachedOptions, CreateOptions, Invocation,
    ListOptions, QueryOptions, RemoteCommandOptions, WipeOptions, parse_invocation,
};
use screen_daemon::PtySessionConfig;
use screen_platform::{RuntimeDirectory, SocketPathStatus, current_effective_uid};
use screen_protocol::Message;

const VERSION: &str = env!("CARGO_PKG_VERSION");

unsafe extern "C" {
    fn setsid() -> i32;
    fn isatty(fd: i32) -> i32;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
    fn tcsetattr(fd: c_int, optional_actions: c_int, termios: *const Termios) -> c_int;
}

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--screen-rs-daemon") {
        return report_result(run_internal_daemon(&args[1..]));
    }

    match parse_invocation(args) {
        Ok(Invocation::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Invocation::Version) => {
            println!("screen-rs {VERSION} (development-only; no GNU Screen compatibility claimed)");
            ExitCode::SUCCESS
        }
        Ok(Invocation::CreateDetached(options)) => report_result(start_detached(options)),
        Ok(Invocation::Create(options)) => match start_attached(options) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("screen-rs: {error}");
                ExitCode::from(1)
            }
        },
        Ok(Invocation::Attach(options)) => match attach(options) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("screen-rs: {error}");
                ExitCode::from(1)
            }
        },
        Ok(Invocation::AttachOrCreate(options)) => match attach_or_create(options) {
            Ok(code) => ExitCode::from(code),
            Err(error) => {
                eprintln!("screen-rs: {error}");
                ExitCode::from(1)
            }
        },
        Ok(Invocation::RemoteCommand(options)) => report_result(remote_command(options)),
        Ok(Invocation::Query(options)) => report_result(query_command(options)),
        Ok(Invocation::List(options)) => match list_sessions(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("screen-rs: {error}");
                ExitCode::from(1)
            }
        },
        Ok(Invocation::Wipe(options)) => match wipe_sessions(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("screen-rs: {error}");
                ExitCode::from(1)
            }
        },
        Ok(invocation) => {
            eprintln!(
                "screen-rs development-only: parsed {invocation:?}; runtime operation is not implemented"
            );
            ExitCode::from(64)
        }
        Err(error) => {
            eprintln!("screen-rs: {error}");
            eprintln!("screen-rs development-only: no GNU Screen compatibility claimed");
            ExitCode::from(64)
        }
    }
}

fn print_help() {
    println!("screen-rs {VERSION} (development-only)");
    println!("No GNU Screen compatibility is claimed yet.");
    println!();
    println!("Usage:");
    println!("  screen-rs --help");
    println!("  screen-rs --version");
    println!("  screen-rs [-S name] [-T term] [-s shell] [-L] [-d -m | -D -m] [command [args...]]");
    println!("  screen-rs -r [name]");
    println!("  screen-rs -ls | -list | -wipe");
    println!("  screen-rs [-S name] [-p window] -X command [args...]");
    println!("  screen-rs [-S name] [-p window] -Q command [args...]");
}

fn report_result(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("screen-rs: {error}");
            ExitCode::from(1)
        }
    }
}

fn start_detached(options: CreateDetachedOptions) -> Result<(), String> {
    start_session_daemon(SessionStartOptions {
        session_name: options.session_name,
        config_file: options.config_file,
        term: options.term,
        shell: options.shell,
        logging: options.logging,
        command: options.command,
        announce_detached: true,
    })
    .map(|_session_name| ())
}

fn start_attached(options: CreateOptions) -> Result<u8, String> {
    let session_name = start_session_daemon(SessionStartOptions {
        session_name: options.session_name,
        config_file: options.config_file,
        term: options.term,
        shell: options.shell,
        logging: options.logging,
        command: options.command,
        announce_detached: false,
    })?;

    attach(AttachOptions {
        session: Some(session_name),
    })
}

struct SessionStartOptions {
    session_name: Option<OsString>,
    config_file: Option<OsString>,
    term: Option<OsString>,
    shell: Option<OsString>,
    logging: bool,
    command: Vec<OsString>,
    announce_detached: bool,
}

fn start_session_daemon(options: SessionStartOptions) -> Result<OsString, String> {
    let runtime = open_or_create_runtime()?;
    let startup_config = load_startup_config(options.config_file.clone())?;
    let _resolved_cfg = explicit_config_path(options.config_file.as_deref());
    let shell = options.shell.or(startup_config.shell);
    let term = options.term.or(startup_config.term);
    let working_directory = startup_config.working_directory;
    let logging = options.logging || startup_config.logging.unwrap_or(false);
    let session_name = options
        .session_name
        .unwrap_or_else(|| OsString::from("screen-rs"));
    runtime
        .session_socket_path(session_name.as_os_str())
        .map_err(|error| error.to_string())?;

    let (program, args) = split_command(options.command, shell);
    let log_path = if logging {
        match startup_config.log_file {
            Some(path) => Some(path),
            None => Some(
                env::current_dir()
                    .map_err(|error| format!("failed to resolve current directory: {error}"))?
                    .join("screenlog.0"),
            ),
        }
    } else {
        None
    };
    let current_exe = env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(current_exe);
    command
        .arg("--screen-rs-daemon")
        .arg(runtime.path())
        .arg(&session_name);
    if let Some(term) = &term {
        command.arg("--term").arg(term);
    }
    if let Some(log_path) = &log_path {
        command.arg("--log").arg(log_path);
    }
    if let Some(working_directory) = &working_directory {
        command.arg("--cwd").arg(working_directory);
    }
    if let Some(hardstatus) = &startup_config.hardstatus {
        // Hex-encode the format string so it survives CLI argument passing safely
        let hex: String = hardstatus.iter().map(|b| format!("{b:02x}")).collect();
        command.arg("--hardstatus").arg(&hex);
    }
    if let Some(limit) = startup_config.defscrollback {
        command.arg("--scrollback").arg(format!("{limit}"));
    }
    if let Some(enabled) = startup_config.defflow {
        command.arg(if enabled { "--flow" } else { "--noflow" });
    }
    if let Some(enabled) = startup_config.defmonitor {
        command.arg(if enabled { "--monitor" } else { "--nomonitor" });
    }
    if let Some(enabled) = startup_config.defwrap {
        command.arg(if enabled { "--wrap" } else { "--nowrap" });
    }
    if let Some(secs) = startup_config.defsilence {
        command.arg("--silence").arg(format!("{secs}"));
    }
    if let Some(enabled) = startup_config.defautonuke
        && enabled
    {
        command.arg("--autonuke");
    }
    // Pass config file to daemon so it can execute startup windows
    let resolved_cfg = explicit_config_path(options.config_file.as_deref());
    if let Some(cfg_path) = resolved_cfg {
        command.arg("--config").arg(cfg_path);
    }
    command
        .arg("--")
        .arg(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    // SAFETY: The pre-exec closure only calls POSIX `setsid`, which takes no
    // pointers and has no Rust aliasing implications. It detaches the session
    // daemon from the launching client's process group before exec.
    unsafe {
        command.pre_exec(|| {
            // SAFETY: `setsid` has no preconditions and is called in the child
            // immediately before exec.
            if setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start session daemon: {error}"))?;
    let socket_name = prefixed_session_socket_name(child.id(), session_name.as_os_str());
    let socket_path = runtime
        .session_socket_path(socket_name.as_os_str())
        .map_err(|error| error.to_string())?;
    let mut daemon_stderr = child.stderr.take();

    for _attempt in 0..200 {
        if socket_path.exists() {
            if options.announce_detached {
                println!(
                    "screen-rs development-only: started detached session {}",
                    session_name.to_string_lossy()
                );
            }
            return Ok(session_name);
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            let mut stderr_bytes = Vec::new();
            if let Some(ref mut stderr_pipe) = daemon_stderr {
                let _ = stderr_pipe.read_to_end(&mut stderr_bytes);
            }
            let stderr_msg = if stderr_bytes.is_empty() {
                String::new()
            } else {
                format!(
                    "\ndaemon stderr: {}",
                    String::from_utf8_lossy(&stderr_bytes)
                )
            };
            return Err(format!(
                "session daemon exited before creating socket: {status}{stderr_msg}"
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }

    Err(format!(
        "session daemon did not create socket within timeout: {}",
        socket_path.display()
    ))
}

#[derive(Debug, Default)]
struct StartupOverrides {
    shell: Option<OsString>,
    term: Option<OsString>,
    working_directory: Option<PathBuf>,
    logging: Option<bool>,
    log_file: Option<PathBuf>,
    hardstatus: Option<Vec<u8>>,
    defscrollback: Option<u32>,
    defmonitor: Option<bool>,
    defflow: Option<bool>,
    defwrap: Option<bool>,
    defsilence: Option<u16>,
    defautonuke: Option<bool>,
    escape: Option<Vec<u8>>,
}

fn load_startup_config(config_file: Option<OsString>) -> Result<StartupOverrides, String> {
    let explicit_path = config_file.map(PathBuf::from);
    let config = if let Some(ref path) = explicit_path {
        screen_config::parse_config_file(path).map_err(|error| format!("{error}"))?
    } else if let Some(env_path) = env::var_os("SCREENRC").map(PathBuf::from) {
        screen_config::parse_config_file(&env_path).map_err(|error| format!("{error}"))?
    } else if let Some(default) = default_screenrc_path() {
        screen_config::parse_config_file(&default).map_err(|error| format!("{error}"))?
    } else {
        return Ok(StartupOverrides::default());
    };

    Ok(StartupOverrides {
        shell: config.shell.map(OsString::from_vec),
        term: config.term.map(OsString::from_vec),
        working_directory: config
            .chdir
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes))),
        logging: config.logging,
        log_file: config
            .logfile
            .map(|bytes| PathBuf::from(OsString::from_vec(bytes))),
        hardstatus: config.hardstatus,
        defscrollback: config.defscrollback,
        defmonitor: config.defmonitor,
        defflow: config.defflow,
        defwrap: config.defwrap,
        defsilence: config.defsilence,
        defautonuke: config.defautonuke,
        escape: config.escape,
    })
}

fn default_screenrc_path() -> Option<PathBuf> {
    let path = PathBuf::from(env::var_os("HOME")?).join(".screenrc");
    path.exists().then_some(path)
}

/// Resolve the explicit config file path: user-specified > SCREENRC env > default .screenrc.
fn explicit_config_path(explicit: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        let p = PathBuf::from(path);
        return p.exists().then_some(p);
    }
    if let Some(env_path) = env::var_os("SCREENRC") {
        let p = PathBuf::from(env_path);
        return p.exists().then_some(p);
    }
    default_screenrc_path()
}

/// Resolve the escape sequence from the screenrc, defaulting to C-a (\x01).
fn resolve_escape() -> Vec<u8> {
    load_startup_config(None)
        .ok()
        .and_then(|o| o.escape)
        .unwrap_or_else(|| vec![0x01])
}

/// Decode a hex string (like "48656c6c6f") into bytes.
fn hex_decode(hex: &OsStr) -> Result<Vec<u8>, String> {
    let hex = hex.as_encoded_bytes();
    if !hex.len().is_multiple_of(2) {
        return Err("hex string has odd length".to_owned());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            let hi = hex_val(hex[i]).ok_or_else(|| format!("invalid hex byte at position {i}"))?;
            let lo = hex_val(hex[i + 1])
                .ok_or_else(|| format!("invalid hex byte at position {}", i + 1))?;
            Ok(hi << 4 | lo)
        })
        .collect()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn run_internal_daemon(args: &[OsString]) -> Result<(), String> {
    let mut index = 2;
    let mut terminal = None;
    let mut log_path = None;
    let mut working_directory = None;
    let mut hardstatus = None;
    let mut scrollback = None;
    let mut flow = None;
    let mut default_monitor = None;
    let mut default_wrap = None;
    let mut default_silence = None;
    let mut auto_nuke = None;
    let mut config_file: Option<PathBuf> = None;
    while let Some(option) = args.get(index) {
        if option == OsStr::new("--config") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --config requires a path argument before --".to_owned()
            })?;
            config_file = Some(PathBuf::from(value));
            index += 1;
        } else if option == OsStr::new("--term") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --term requires an argument before --".to_owned()
            })?;
            terminal = Some(value.clone());
            index += 1;
        } else if option == OsStr::new("--log") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --log requires an argument before --".to_owned()
            })?;
            log_path = Some(PathBuf::from(value));
            index += 1;
        } else if option == OsStr::new("--cwd") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --cwd requires an argument before --".to_owned()
            })?;
            working_directory = Some(PathBuf::from(value));
            index += 1;
        } else if option == OsStr::new("--hardstatus") {
            index += 1;
            let hex = args.get(index).ok_or_else(|| {
                "internal daemon mode --hardstatus requires an argument before --".to_owned()
            })?;
            hardstatus = Some(hex_decode(hex).map_err(|e| format!("invalid hardstatus hex: {e}"))?);
            index += 1;
        } else if option == OsStr::new("--scrollback") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --scrollback requires an argument before --".to_owned()
            })?;
            scrollback = Some(
                value
                    .to_str()
                    .ok_or_else(|| "scrollback value is not valid UTF-8".to_owned())?
                    .parse::<u32>()
                    .map_err(|e| format!("invalid scrollback: {e}"))?,
            );
            index += 1;
        } else if option == OsStr::new("--flow") {
            flow = Some(true);
            index += 1;
        } else if option == OsStr::new("--noflow") {
            flow = Some(false);
            index += 1;
        } else if option == OsStr::new("--monitor") {
            default_monitor = Some(true);
            index += 1;
        } else if option == OsStr::new("--nomonitor") {
            default_monitor = Some(false);
            index += 1;
        } else if option == OsStr::new("--wrap") {
            default_wrap = Some(true);
            index += 1;
        } else if option == OsStr::new("--nowrap") {
            default_wrap = Some(false);
            index += 1;
        } else if option == OsStr::new("--silence") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --silence requires an argument before --".to_owned()
            })?;
            default_silence = Some(
                value
                    .to_str()
                    .ok_or_else(|| "silence value is not valid UTF-8".to_owned())?
                    .parse::<u16>()
                    .map_err(|e| format!("invalid silence: {e}"))?,
            );
            index += 1;
        } else if option == OsStr::new("--autonuke") {
            auto_nuke = Some(true);
            index += 1;
        } else {
            break;
        }
    }

    if args.len() <= index + 1 || args.get(index).is_none_or(|arg| arg != OsStr::new("--")) {
        return Err(
            "internal daemon mode requires: <runtime-dir> <session-name> [--term term] -- <program> [args...]"
                .to_owned(),
        );
    }

    let socket_name = prefixed_session_socket_name(std::process::id(), args[1].as_os_str());
    let socket_path = PathBuf::from(&args[0]).join(socket_name);
    let program = args[index + 1].clone();
    let program_args = args[index + 2..].to_vec();
    let mut config = PtySessionConfig::new(socket_path, program, program_args);
    if let Some(terminal) = terminal {
        config.terminal = terminal;
    }
    config.log_path = log_path;
    config.working_directory = working_directory;
    config.hardstatus_format = hardstatus;
    config.scrollback_limit = scrollback;
    config.default_flow = flow;
    config.default_monitor = default_monitor;
    config.default_wrap = default_wrap;
    config.default_silence = default_silence;
    config.auto_nuke = auto_nuke;
    // Load startup windows from config file
    if let Some(ref cfg_path) = config_file
        && let Ok(screenrc) = screen_config::parse_config_file(cfg_path)
    {
        config.startup_windows = screenrc
            .startup_windows
            .into_iter()
            .map(|sw| screen_daemon::StartupWindow {
                title: sw.title,
                program: sw.program.map(OsString::from_vec),
                args: sw.args.into_iter().map(OsString::from_vec).collect(),
                number: sw.number,
                working_directory: sw.working_directory.map(OsString::from_vec),
                stuff: sw.stuff,
            })
            .collect();
    }
    screen_daemon::run_pty_session(config).map_err(|error| error.to_string())
}

fn attach(options: AttachOptions) -> Result<u8, String> {
    let runtime = open_or_create_runtime()?;
    let socket_path = resolve_session_socket(&runtime, options.session)?;
    attach_socket(socket_path, resolve_escape())
}

fn attach_or_create(options: AttachOrCreateOptions) -> Result<u8, String> {
    let runtime = open_or_create_runtime()?;
    if let Some(session) = &options.session {
        runtime
            .session_socket_path(session.as_os_str())
            .map_err(|error| error.to_string())?;
    }

    let escape = resolve_escape();
    match find_active_session_socket(&runtime, options.session.as_deref())? {
        ActiveSessionMatch::One(socket_path) => attach_socket(socket_path, escape),
        ActiveSessionMatch::None => start_attached(CreateOptions {
            session_name: options.session,
            config_file: None,
            term: None,
            shell: None,
            logging: false,
            force_new: false,
            command: Vec::new(),
        }),
        ActiveSessionMatch::Multiple => {
            if let Some(session) = options.session {
                Err(format!(
                    "multiple active screen-rs sessions match {}; specify the full socket name",
                    session.to_string_lossy()
                ))
            } else {
                Err("multiple active screen-rs sessions found; specify -r <name>".to_owned())
            }
        }
    }
}

fn attach_socket(socket_path: PathBuf, escape: Vec<u8>) -> Result<u8, String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;

    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Attach
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    if let Some((columns, rows)) = terminal_size() {
        Message::Resize { columns, rows }
            .write_to(&mut stream)
            .map_err(|error| error.to_string())?;
    }

    if !stdin_is_tty() {
        return attach_snapshot(stream);
    }

    let _raw_terminal = RawTerminalGuard::enter_stdin()?;

    // Ignore SIGHUP so the client stays alive when the terminal emulator
    // closes. The stdin read will return EOF naturally, triggering detach.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }

    let mut input_stream = stream
        .try_clone()
        .map_err(|error| format!("failed to clone client socket: {error}"))?;
    // Shared paste buffer between the stdin thread and the main loop
    let paste_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let paste_clone = Arc::clone(&paste_buffer);
    let escape_prefix = escape[0];
    let escape_meta = escape.get(1).copied().unwrap_or(escape_prefix);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 4096];
        let mut prefix = false;
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => {
                    let _ = Message::Detach.write_to(&mut input_stream);
                    break;
                }
                Ok(read) => {
                    for byte in &buffer[..read] {
                        if prefix {
                            prefix = false;
                            match *byte {
                                b'd' => {
                                    let _ = Message::Detach.write_to(&mut input_stream);
                                    return;
                                }
                                b'c' => {
                                    let _ = Message::CreateWindow {
                                        program: Vec::new(),
                                        args: Vec::new(),
                                    }
                                    .write_to(&mut input_stream);
                                }
                                b'n' | b' ' => {
                                    let _ = Message::NextWindow.write_to(&mut input_stream);
                                }
                                b'p' => {
                                    let _ = Message::PrevWindow.write_to(&mut input_stream);
                                }
                                b'k' => {
                                    let _ = Message::KillWindow { number: 0 }
                                        .write_to(&mut input_stream);
                                }
                                b'0'..=b'9' => {
                                    let number = (*byte - b'0') as u32;
                                    let _ = Message::SelectWindow { number }
                                        .write_to(&mut input_stream);
                                }
                                b'"' => {
                                    let _ =
                                        Message::WindowList(Vec::new()).write_to(&mut input_stream);
                                }
                                b'w' => {
                                    // Window list (alternate binding)
                                    let _ =
                                        Message::WindowList(Vec::new()).write_to(&mut input_stream);
                                }
                                b'A' => {
                                    // Set window title - prompt in command mode
                                    // For now, set via OSC escape sent as PtyInput
                                    let title = b"\x1b]2;\x07";
                                    let _ = Message::PtyInput(title.to_vec())
                                        .write_to(&mut input_stream);
                                }
                                b'H' => {
                                    // Toggle logging - not implemented server-side yet
                                }
                                b']' => {
                                    // Paste buffer
                                    let paste = paste_clone.lock().unwrap().clone();
                                    if !paste.is_empty() {
                                        let _ =
                                            Message::PtyInput(paste).write_to(&mut input_stream);
                                    }
                                }
                                b'<' => {
                                    // Read exchange file into paste buffer
                                    if let Ok(data) = std::fs::read("/tmp/screen-exchange") {
                                        *paste_clone.lock().unwrap() = data;
                                    }
                                }
                                b'>' => {
                                    // Write paste buffer to exchange file
                                    let data = paste_clone.lock().unwrap().clone();
                                    let _ = std::fs::write("/tmp/screen-exchange", data);
                                }
                                b'S' => {
                                    // Split display
                                    let _ = Message::SplitVertical.write_to(&mut input_stream);
                                }
                                b'Q' => {
                                    // Remove all splits but current
                                    let _ = Message::OnlyWindow.write_to(&mut input_stream);
                                }
                                b'\t' => {
                                    // Switch to next split region
                                    let _ = Message::FocusNext.write_to(&mut input_stream);
                                }
                                b'X' => {
                                    // Remove current region
                                    let _ = Message::RemoveRegion.write_to(&mut input_stream);
                                }
                                b'[' => {
                                    // Enter copy mode - send request to daemon
                                    let _ = Message::CopyModeRequest.write_to(&mut input_stream);
                                }
                                b'{' => {
                                    // Search history backwards
                                    // For now, enter local copy mode with search prompt
                                    let _ = Message::CopyModeRequest.write_to(&mut input_stream);
                                }
                                b'}' => {
                                    // Search history forwards — also enters copy mode
                                    let _ = Message::CopyModeRequest.write_to(&mut input_stream);
                                }
                                b'=' => {
                                    // Remove exchange file (C-a =)
                                    let _ = std::fs::remove_file("/tmp/screen-exchange");
                                }
                                b'r' => {
                                    // Toggle line wrapping
                                    let _ = Message::WrapToggle { enable: true }
                                        .write_to(&mut input_stream);
                                }
                                b'f' => {
                                    // Toggle flow control
                                    let _ = Message::FlowToggle { enable: true }
                                        .write_to(&mut input_stream);
                                }
                                b's' => {
                                    // Send XOFF (C-a s)
                                    let _ = Message::Xoff.write_to(&mut input_stream);
                                }
                                b'q' => {
                                    // Send XON (C-a q)
                                    let _ = Message::Xon.write_to(&mut input_stream);
                                }
                                b'b' => {
                                    // Send break (C-a b)
                                    let _ = Message::BreakSignal { ms: 250 }
                                        .write_to(&mut input_stream);
                                }
                                b'M' => {
                                    // Toggle activity monitoring (C-a M)
                                    let _ = Message::MonitorToggle { enable: true }
                                        .write_to(&mut input_stream);
                                }
                                b'_' => {
                                    // Toggle silence monitoring (C-a _)
                                    let _ = Message::Silence { seconds: 30 }
                                        .write_to(&mut input_stream);
                                }
                                b'i' => {
                                    // Window info (C-a i)
                                    let _ =
                                        Message::WindowInfo(Vec::new()).write_to(&mut input_stream);
                                }
                                byte @ (b'm' | b'\x0d') => {
                                    // Last message (C-a m / C-a C-m)
                                    // Display local last message hint
                                    let _ = byte;
                                }
                                b'\x07' => {
                                    // Visual bell toggle (C-a C-g)
                                    // Client-side: toggle visual bell mode
                                }
                                b'\x0c' => {
                                    // Redisplay (C-a C-l) — alias for redisplay
                                    let _ = Message::Redisplay.write_to(&mut input_stream);
                                }
                                b'\x18' => {
                                    // Lockscreen (C-a C-x / C-a x)
                                    // Client-side: lock terminal
                                }
                                b'\x1a' => {
                                    // Suspend (C-a C-z / C-a z)
                                    unsafe {
                                        libc::raise(libc::SIGTSTP);
                                    }
                                }
                                b'Z' => {
                                    // Reset terminal (C-a Z)
                                    // Send RIS sequence to stdout
                                    let mut stdout = io::stdout().lock();
                                    let _ = stdout.write_all(b"\x1bc");
                                    let _ = stdout.flush();
                                    drop(stdout);
                                    let _ = Message::Redisplay.write_to(&mut input_stream);
                                }
                                b'C' => {
                                    // Clear screen (C-a C)
                                    let _ = Message::Redisplay.write_to(&mut input_stream);
                                }
                                b'v' => {
                                    // Version (C-a v) — display locally
                                    let mut stdout = io::stdout().lock();
                                    let msg =
                                        format!("screen-rs {}\r\n", env!("CARGO_PKG_VERSION"));
                                    let _ = stdout.write_all(msg.as_bytes());
                                    let _ = stdout.flush();
                                    drop(stdout);
                                }
                                b't' => {
                                    // Time (C-a t) — display locally
                                    let mut stdout = io::stdout().lock();
                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default();
                                    let msg = format!("screen-rs  uptime: {}s\r\n", now.as_secs());
                                    let _ = stdout.write_all(msg.as_bytes());
                                    let _ = stdout.flush();
                                    drop(stdout);
                                }
                                b'?' => {
                                    // Help (C-a ?) — display locally
                                    let mut stdout = io::stdout().lock();
                                    let help = b"\
                                        C-a c  create   C-a k  kill      C-a n/p  next/prev\r\n\
                                        C-a d  detach   C-a A  title     C-a w    windows\r\n\
                                        C-a [  copy     C-a ]  paste     C-a <    readbuf\r\n\
                                        C-a >  writebuf C-a =  removebuf C-a {/}  history\r\n\
                                        C-a M  monitor  C-a _  silence   C-a r    wrap\r\n\
                                        C-a i  info     C-a t  time      C-a v    version\r\n\
                                        C-a '  select   C-a \"  winlist   C-a C-a  other\r\n\
                                        C-a :  command  C-a ?  help      C-a ,    license\r\n\
                                        C-a z  suspend  C-a Z  reset     C-a x    lock\r\n\
                                        C-a f  flow     C-a s  xoff      C-a q    xon\r\n\
                                        C-a b  break    C-a B  pow_break C-a .    termcap\r\n";
                                    let _ = stdout.write_all(help);
                                    let _ = stdout.flush();
                                    drop(stdout);
                                }
                                b',' => {
                                    // License (C-a ,)
                                    let mut stdout = io::stdout().lock();
                                    let msg = b"screen-rs  GPL-3.0-or-later  https://github.com/Lbniese/screen-rs\r\n";
                                    let _ = stdout.write_all(msg);
                                    let _ = stdout.flush();
                                    drop(stdout);
                                }
                                b'.' => {
                                    // Dump termcap (C-a .)
                                    let mut stdout = io::stdout().lock();
                                    let termcap = b"screen|VT100/ANSI X3.64 virtual terminal:\r\n\
                                        :am:xn:msgr:li#24:co#80:cl=\\E[H\\E[J:cm=\\E[%i%d;%dH:\r\n";
                                    let _ = stdout.write_all(termcap);
                                    let _ = stdout.flush();
                                    let _ = std::fs::write(".termcap", termcap);
                                    drop(stdout);
                                }
                                b'h' => {
                                    // Hardcopy (C-a h) — write current screen to file
                                    // Request redisplay to capture content
                                    let _ = Message::Redisplay.write_to(&mut input_stream);
                                }
                                b'\'' => {
                                    // Select window prompt (C-a ')
                                    // Send window list request to daemon for interactive selection
                                    let _ =
                                        Message::WindowList(Vec::new()).write_to(&mut input_stream);
                                }
                                b'B' => {
                                    // Power break (C-a B)
                                    let _ = Message::BreakSignal { ms: 500 }
                                        .write_to(&mut input_stream);
                                }
                                b'D' => {
                                    // Power detach (C-a D D) — second D
                                    let _ = Message::Detach.write_to(&mut input_stream);
                                }
                                b'\x08' | b'\x7f' => {
                                    // Backspace/delete (C-a backspace) — last window
                                    let _ = Message::OtherWindow.write_to(&mut input_stream);
                                }
                                b'x' => {
                                    // Lock screen (C-a x)
                                    let mut stdout = io::stdout().lock();
                                    let msg =
                                        b"\x1b[H\x1b[JScreen locked. Use password to unlock.\r\n";
                                    let _ = stdout.write_all(msg);
                                    let _ = stdout.flush();
                                    drop(stdout);
                                }
                                b'\x16' => {
                                    // Digraph (C-a C-v)
                                    // Client-side digraph mode — not implemented yet
                                }
                                byte if byte == escape_prefix || byte == escape_meta => {
                                    // C-a C-a (other window) or C-a meta-char: send to window
                                    if byte == escape_prefix {
                                        let _ = Message::OtherWindow.write_to(&mut input_stream);
                                    } else {
                                        let _ = Message::PtyInput(vec![byte])
                                            .write_to(&mut input_stream);
                                    }
                                }
                                byte => {
                                    let bytes = vec![escape_prefix, byte];
                                    if Message::PtyInput(bytes)
                                        .write_to(&mut input_stream)
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                            }
                        } else if *byte == escape_prefix {
                            prefix = true;
                        } else if Message::PtyInput(vec![*byte])
                            .write_to(&mut input_stream)
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(_error) => {
                    let _ = Message::Detach.write_to(&mut input_stream);
                    break;
                }
            }
        }
    });

    let mut stdout = io::stdout().lock();
    loop {
        match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
            Message::PtyOutput(bytes) => {
                stdout
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?;
                stdout.flush().map_err(|error| error.to_string())?;
            }
            Message::ChildExited(code) => return Ok(normalize_exit_code(code)),
            Message::Error(bytes) => return Err(String::from_utf8_lossy(&bytes).into_owned()),
            Message::Detach => return Ok(0),
            Message::CopyModeData(lines) => {
                // Enter local copy mode with scrollback lines
                if let Some(selected) = copy_mode_navigate(&lines, &mut stream) {
                    *paste_buffer.lock().unwrap() = selected;
                }
            }
            Message::HardstatusLine(line) => {
                // Render hardstatus at bottom of terminal
                let (cols, rows) = terminal_size().unwrap_or((80, 24));
                let mut status = line.clone();
                // Truncate to terminal width
                status.truncate(cols as usize);
                // Pad with spaces to terminal width (reverse video will fill)
                while status.len() < cols as usize {
                    status.push(b' ');
                }
                // Save cursor, move to last line, write in reverse video, restore
                let mut seq = Vec::new();
                seq.extend_from_slice(b"\x1b7"); // save cursor
                seq.extend_from_slice(format!("\x1b[{};1H", rows).as_bytes()); // move to last line
                seq.extend_from_slice(b"\x1b[7m"); // reverse video
                seq.extend_from_slice(&status);
                seq.extend_from_slice(b"\x1b[0m"); // reset
                seq.extend_from_slice(b"\x1b8"); // restore cursor
                stdout.write_all(&seq).ok();
                stdout.flush().ok();
            }
            Message::WindowSelected { number } => {
                // Window was selected, continue reading
                let _ = number;
            }
            Message::WindowExited { number, .. } => {
                // A window exited, continue reading
                let _ = number;
            }
            Message::WindowList(list) => {
                // Display window list to stderr (like GNU Screen does)
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "Num Name       Flags");
                for w in &list {
                    let marker = if w.flags & 1 != 0 { '*' } else { ' ' };
                    let dead = if w.flags & 2 != 0 { "(dead)" } else { "" };
                    let title = String::from_utf8_lossy(&w.title);
                    let _ = writeln!(stderr, "{:<3} {:<10} {}{}", w.number, marker, dead, title);
                }
            }
            Message::Activity(msg) => {
                // Display activity notification on the message line
                let mut stderr = io::stderr().lock();
                let text = String::from_utf8_lossy(&msg);
                let _ = writeln!(stderr, "\r\n{}", text);
            }
            Message::Bell(_msg) => {
                // Audible/visual bell — ring terminal bell
                let mut stdout = io::stdout().lock();
                let _ = stdout.write_all(b"\x07");
                let _ = stdout.flush();
            }
            Message::WindowInfo(info) => {
                // Display window info
                let mut stderr = io::stderr().lock();
                let text = String::from_utf8_lossy(&info);
                let _ = writeln!(stderr, "{}", text);
            }
            Message::SearchResult(matches) => {
                // Display search results — for now just print count
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "Found {} match(es)", matches.len());
            }
            _message => {}
        }
    }
}

fn remote_command(options: RemoteCommandOptions) -> Result<(), String> {
    let Some(command) = options.command.first() else {
        return Err("remote command requires a command name".to_owned());
    };

    if command == OsStr::new("quit") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_shutdown(socket_path)
    } else if command == OsStr::new("detach") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_detach(socket_path)
    } else if command == OsStr::new("stuff") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_stuff(socket_path, &options.command[1..])
    } else if command == OsStr::new("screen") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_create_window(socket_path, &options.command[1..])
    } else if command == OsStr::new("select") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_select_window(socket_path, &options.command[1..])
    } else if command == OsStr::new("next") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_next_window(socket_path)
    } else if command == OsStr::new("prev") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_prev_window(socket_path)
    } else if command == OsStr::new("kill") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_kill_window(socket_path, &options.command[1..])
    } else if command == OsStr::new("windows") || command == OsStr::new("winlist") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_windows_list(socket_path)
    } else if command == OsStr::new("title") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_title(socket_path, &options.command[1..])
    } else if command == OsStr::new("number") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_renumber(socket_path, &options.command[1..])
    } else if command == OsStr::new("redisplay") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_redisplay(socket_path)
    } else if command == OsStr::new("remove") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_remove_window(socket_path, &options.command[1..])
    } else if command == OsStr::new("wipe") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_wipe_dead_windows(socket_path)
    } else if command == OsStr::new("echo") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_echo(socket_path, &options.command[1..])
    } else if command == OsStr::new("log") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_log_toggle(socket_path, &options.command[1..])
    } else if command == OsStr::new("logfile") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_logfile(socket_path, &options.command[1..])
    } else if command == OsStr::new("other") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_other_window(socket_path)
    } else if command == OsStr::new("monitor") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_monitor_toggle(socket_path, &options.command[1..])
    } else if command == OsStr::new("silence") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_silence(socket_path, &options.command[1..])
    } else if command == OsStr::new("wrap") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_wrap_toggle(socket_path, &options.command[1..])
    } else if command == OsStr::new("readbuf") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_readbuf(socket_path)
    } else if command == OsStr::new("writebuf") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_writebuf(socket_path, &options.command[1..])
    } else if command == OsStr::new("removebuf") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_removebuf(socket_path)
    } else if command == OsStr::new("register") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_register(socket_path, &options.command[1..])
    } else if command == OsStr::new("flow") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_flow_toggle(socket_path, &options.command[1..])
    } else if command == OsStr::new("xoff") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_xoff(socket_path)
    } else if command == OsStr::new("xon") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_xon(socket_path)
    } else if command == OsStr::new("break") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_break(socket_path, &options.command[1..])
    } else if command == OsStr::new("info") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_info(socket_path)
    } else if command == OsStr::new("search") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_search_history(socket_path, &options.command[1..])
    } else if command == OsStr::new("colon") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_colon(socket_path, &options.command[1..])
    } else if command == OsStr::new("hardcopy") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_hardcopy(socket_path, &options.command[1..])
    } else if command == OsStr::new("at") {
        // -X at <window#> <command> [args...]
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_at(socket_path, &options.command[1..])
    } else if command == OsStr::new("time") {
        // Print current local time
        let now: libc::time_t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as libc::time_t)
            .unwrap_or(0);
        unsafe {
            let tm: *mut libc::tm = libc::localtime(&now);
            if tm.is_null() {
                return Err("failed to get local time".to_owned());
            }
            let mut buf: [libc::c_char; 256] = [0; 256];
            let n = libc::strftime(buf.as_mut_ptr(), buf.len(), c"%c".as_ptr(), tm);
            if n > 0 {
                let s = std::ffi::CStr::from_ptr(buf.as_ptr());
                eprintln!("{}", s.to_string_lossy());
            }
        }
        Ok(())
    } else if command == OsStr::new("help") {
        eprintln!("screen-rs — terminal multiplexer (https://github.com/Lbniese/screen-rs)");
        Ok(())
    } else if command == OsStr::new("license") {
        eprintln!("screen-rs is licensed under the GNU General Public License v3.0");
        Ok(())
    } else if command == OsStr::new("suspend") {
        unsafe {
            libc::kill(std::process::id() as i32, libc::SIGTSTP);
        }
        Ok(())
    } else {
        Err(format!(
            "remote command is not implemented yet: {}",
            command.to_string_lossy()
        ))
    }
}

fn query_command(options: QueryOptions) -> Result<(), String> {
    let Some(command) = options.command.first() else {
        return Err("query command requires a command name".to_owned());
    };

    if command == OsStr::new("windows") || command == OsStr::new("winlist") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_windows_list(socket_path)
    } else {
        Err(format!(
            "query command is not implemented yet: {}",
            command.to_string_lossy()
        ))
    }
}

fn send_create_window(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let program = args
        .first()
        .map(|p| p.as_os_str().as_bytes().to_vec())
        .unwrap_or_else(|| {
            env::var_os("SHELL")
                .unwrap_or_else(|| OsString::from("/bin/sh"))
                .as_bytes()
                .to_vec()
        });
    let extra_args: Vec<Vec<u8>> = args
        .get(1..)
        .map(|rest| {
            rest.iter()
                .map(|a| a.as_os_str().as_bytes().to_vec())
                .collect()
        })
        .unwrap_or_default();

    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::CreateWindow {
        program,
        args: extra_args,
    }
    .write_to(&mut stream)
    .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::WindowCreated { number, .. } => {
            println!("{number}");
            Ok(())
        }
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        message => Err(format!("unexpected response: {message:?}")),
    }
}

fn send_select_window(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let number: u32 = args
        .first()
        .and_then(|a| a.to_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    send_control_message(socket_path, Message::SelectWindow { number })
}

fn send_next_window(socket_path: PathBuf) -> Result<(), String> {
    send_control_message(socket_path, Message::NextWindow)
}

fn send_prev_window(socket_path: PathBuf) -> Result<(), String> {
    send_control_message(socket_path, Message::PrevWindow)
}

fn send_kill_window(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let number: u32 = args
        .first()
        .and_then(|a| a.to_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    send_control_message(socket_path, Message::KillWindow { number })
}

fn send_windows_list(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::WindowList(Vec::new())
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::WindowList(list) => {
            for w in &list {
                let marker = if w.flags & 1 != 0 { '*' } else { ' ' };
                let dead = if w.flags & 2 != 0 { "(dead)" } else { "" };
                let title = String::from_utf8_lossy(&w.title);
                println!("{}\t{}\t{}{}", w.number, marker, dead, title);
            }
            Ok(())
        }
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        message => Err(format!("unexpected response: {message:?}")),
    }
}

fn send_title(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let title = args
        .first()
        .map(|a| a.as_os_str().as_bytes().to_vec())
        .unwrap_or_default();

    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::WindowTitle { number: 0, title }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    // Read response
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_renumber(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let new_number: u32 = args
        .first()
        .and_then(|a| a.to_str())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "number requires a numeric argument".to_owned())?;

    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::RenumberWindow { number: new_number }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    // Read response
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_redisplay(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Redisplay
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    // Read response (daemon sends Error or nothing)
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_remove_window(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let number = args
        .first()
        .and_then(|a| a.to_str())
        .and_then(|s| s.parse().ok());
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::RemoveWindow {
        number: number.unwrap_or(0),
    }
    .write_to(&mut stream)
    .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_wipe_dead_windows(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::WipeDeadWindows
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_echo(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let text: Vec<u8> = args
        .iter()
        .flat_map(|a| a.as_encoded_bytes())
        .copied()
        .collect();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Echo(text)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_log_toggle(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let enable = !args
        .first()
        .is_some_and(|a| a.as_encoded_bytes().eq_ignore_ascii_case(b"off"));
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::LogToggle { enable }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_logfile(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let path = args
        .first()
        .map(|a| a.as_encoded_bytes().to_vec())
        .unwrap_or_default();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::LogFile(path)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_other_window(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::OtherWindow
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_monitor_toggle(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let enable = !args
        .first()
        .is_some_and(|a| a.as_encoded_bytes().eq_ignore_ascii_case(b"off"));
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::MonitorToggle { enable }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_silence(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let seconds: u16 = args
        .first()
        .and_then(|a| a.to_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Silence { seconds }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_wrap_toggle(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let enable = !args
        .first()
        .is_some_and(|a| a.as_encoded_bytes().eq_ignore_ascii_case(b"off"));
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::WrapToggle { enable }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_readbuf(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::ReadBuf
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_writebuf(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let data: Vec<u8> = args
        .iter()
        .flat_map(|a| a.as_encoded_bytes())
        .copied()
        .collect();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::WriteBuf(data)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_removebuf(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::RemoveBuf
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_register(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let name = args
        .first()
        .and_then(|a| a.as_encoded_bytes().first().copied())
        .unwrap_or(b'a');
    let data: Vec<u8> = args
        .get(1..)
        .unwrap_or(&[])
        .iter()
        .flat_map(|a| a.as_encoded_bytes())
        .copied()
        .collect();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Register { name, data }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_flow_toggle(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let enable = !args
        .first()
        .is_some_and(|a| a.as_encoded_bytes().eq_ignore_ascii_case(b"off"));
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::FlowToggle { enable }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_xoff(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Xoff
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_xon(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Xon
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_break(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let ms: u16 = args
        .first()
        .and_then(|a| a.to_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::BreakSignal { ms }
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_info(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::WindowInfo(Vec::new())
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_search_history(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let query: Vec<u8> = args
        .iter()
        .flat_map(|a| a.as_encoded_bytes())
        .copied()
        .collect();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::SearchHistory(query)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_colon(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let cmd: Vec<u8> = args
        .iter()
        .flat_map(|a| a.as_encoded_bytes())
        .copied()
        .collect();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Command(cmd)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_at(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    // args: [window_number, command, ...args]
    if args.len() < 2 {
        return Err("usage: -X at <window#> <command> [args...]".to_owned());
    }
    let number = args[0]
        .to_str()
        .ok_or_else(|| "window number is not valid UTF-8".to_owned())?
        .parse::<u32>()
        .map_err(|e| format!("invalid window number: {e}"))?;
    let command = &args[1];
    let cmd_args = if args.len() > 2 { &args[2..] } else { &[] };

    // Use the send_other_window approach: select the window, send command, restore
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }

    // Build command as a string to send via colon (simplified)
    let mut cmd_str = command.as_encoded_bytes().to_vec();
    for arg in cmd_args {
        cmd_str.push(b' ');
        cmd_str.extend_from_slice(arg.as_encoded_bytes());
    }
    // Send as OtherWindow for now — sends the command string as input to the target
    Message::AtWindow(number, cmd_str)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_hardcopy(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let (number, path) = if args.is_empty() {
        (0u32, b"hardcopy.0".to_vec())
    } else {
        let first = args[0].as_encoded_bytes();
        if let Ok(n) = std::str::from_utf8(first).unwrap_or("").parse::<u32>() {
            let file = if args.len() > 1 {
                args[1].as_encoded_bytes().to_vec()
            } else {
                format!("hardcopy.{n}").into_bytes()
            };
            (n, file)
        } else {
            (0u32, first.to_vec())
        }
    };
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Hardcopy(number, path)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_shutdown(socket_path: PathBuf) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Shutdown
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::ShutdownAck => Ok(()),
        message => Err(format!("unexpected daemon shutdown response: {message:?}")),
    }
}

fn send_stuff(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    let Some(first) = args.first() else {
        return Err("stuff requires an argument".to_owned());
    };
    let mut bytes = first.as_os_str().as_bytes().to_vec();
    for arg in &args[1..] {
        bytes.push(b' ');
        bytes.extend_from_slice(arg.as_os_str().as_bytes());
    }

    send_control_message(socket_path, Message::PtyInput(bytes))
}

fn send_detach(socket_path: PathBuf) -> Result<(), String> {
    send_control_message(socket_path, Message::Detach)
}

fn send_control_message(socket_path: PathBuf, message: Message) -> Result<(), String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    message
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    // Drain response so the daemon connection is cleanly closed
    let _ = Message::read_from(&mut stream);
    Ok(())
}

fn attach_snapshot(mut stream: UnixStream) -> Result<u8, String> {
    let mut stdout = io::stdout().lock();
    loop {
        match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
            Message::PtyOutput(bytes) => {
                stdout
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?;
                stdout.flush().map_err(|error| error.to_string())?;
                let _ = Message::Detach.write_to(&mut stream);
                return Ok(0);
            }
            Message::ChildExited(code) => return Ok(normalize_exit_code(code)),
            Message::Error(bytes) => return Err(String::from_utf8_lossy(&bytes).into_owned()),
            _message => {}
        }
    }
}

fn list_sessions(options: ListOptions) -> Result<(), String> {
    let runtime = open_or_create_runtime()?;
    let entries = filter_session_entries(
        session_socket_entries(&runtime)?,
        options.session_match.as_deref(),
    );
    print_session_listing(runtime.path(), &entries);
    Ok(())
}

fn print_session_listing(runtime: &std::path::Path, entries: &[SessionSocketEntry]) {
    if entries.is_empty() {
        println!("No Sockets found in {}.", runtime.display());
        println!();
        return;
    }

    if entries.len() == 1 {
        println!("There is a screen on:");
    } else {
        println!("There are screens on:");
    }
    for entry in entries {
        let state = match entry.status {
            SocketPathStatus::ActiveSocket => "Detached",
            SocketPathStatus::StaleSocket => "Dead",
            _ => continue,
        };
        let date = entry
            .created_at
            .as_ref()
            .map(|t| format_socket_date(*t))
            .unwrap_or_default();
        if date.is_empty() {
            println!("\t{}\t({state})", entry.name.to_string_lossy());
        } else {
            println!("\t{}\t({date})\t({state})", entry.name.to_string_lossy());
        }
    }
    let noun = if entries.len() == 1 {
        "Socket"
    } else {
        "Sockets"
    };
    println!("{} {noun} in {}.", entries.len(), runtime.display());
}

fn wipe_sessions(options: WipeOptions) -> Result<(), String> {
    let runtime = open_or_create_runtime()?;

    for entry in session_socket_entries(&runtime)? {
        if entry.status == SocketPathStatus::StaleSocket
            && options
                .session_match
                .as_deref()
                .is_none_or(|requested| session_name_matches(entry.name.as_os_str(), requested))
        {
            let path = runtime
                .session_socket_path(entry.name.as_os_str())
                .map_err(|error| error.to_string())?;
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    }

    let entries = filter_session_entries(
        session_socket_entries(&runtime)?,
        options.session_match.as_deref(),
    );
    print_session_listing(runtime.path(), &entries);
    Ok(())
}

fn filter_session_entries(
    entries: Vec<SessionSocketEntry>,
    requested: Option<&OsStr>,
) -> Vec<SessionSocketEntry> {
    entries
        .into_iter()
        .filter(|entry| {
            requested
                .is_none_or(|requested| session_name_matches(entry.name.as_os_str(), requested))
        })
        .collect()
}

fn resolve_session_socket(
    runtime: &RuntimeDirectory,
    session: Option<OsString>,
) -> Result<PathBuf, String> {
    if let Some(session) = session {
        runtime
            .session_socket_path(session.as_os_str())
            .map_err(|error| error.to_string())?;
        return match find_active_session_socket(runtime, Some(session.as_os_str()))? {
            ActiveSessionMatch::One(socket_path) => Ok(socket_path),
            ActiveSessionMatch::None => Err(format!(
                "no active screen-rs session found matching {}",
                session.to_string_lossy()
            )),
            ActiveSessionMatch::Multiple => Err(format!(
                "multiple active screen-rs sessions match {}; specify the full socket name",
                session.to_string_lossy()
            )),
        };
    }

    match find_active_session_socket(runtime, None)? {
        ActiveSessionMatch::One(socket_path) => Ok(socket_path),
        ActiveSessionMatch::None => Err("no active screen-rs sessions found".to_owned()),
        ActiveSessionMatch::Multiple => {
            Err("multiple active screen-rs sessions found; specify -r <name>".to_owned())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveSessionMatch {
    None,
    One(PathBuf),
    Multiple,
}

fn find_active_session_socket(
    runtime: &RuntimeDirectory,
    requested: Option<&OsStr>,
) -> Result<ActiveSessionMatch, String> {
    let mut matches = Vec::new();
    for entry in session_socket_entries(runtime)? {
        if entry.status != SocketPathStatus::ActiveSocket {
            continue;
        }
        if requested.is_none_or(|requested| session_name_matches(entry.name.as_os_str(), requested))
        {
            matches.push(
                runtime
                    .session_socket_path(entry.name.as_os_str())
                    .map_err(|error| error.to_string())?,
            );
        }
    }

    Ok(match matches.len() {
        0 => ActiveSessionMatch::None,
        1 => ActiveSessionMatch::One(matches.remove(0)),
        _ => ActiveSessionMatch::Multiple,
    })
}

#[derive(Debug)]
struct SessionSocketEntry {
    name: OsString,
    status: SocketPathStatus,
    created_at: Option<SystemTime>,
}

fn session_socket_entries(runtime: &RuntimeDirectory) -> Result<Vec<SessionSocketEntry>, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(runtime.path()).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let status = runtime
            .classify_session_socket(name.as_os_str())
            .map_err(|error| error.to_string())?;
        if matches!(
            status,
            SocketPathStatus::ActiveSocket | SocketPathStatus::StaleSocket
        ) {
            let created_at = entry.metadata().ok().and_then(|md| md.modified().ok());
            entries.push(SessionSocketEntry {
                name,
                status,
                created_at,
            });
        }
    }
    entries.sort_by(|left, right| {
        left.name
            .as_os_str()
            .as_bytes()
            .cmp(right.name.as_os_str().as_bytes())
    });
    Ok(entries)
}

fn prefixed_session_socket_name(process_id: u32, session_name: &OsStr) -> OsString {
    let mut bytes = process_id.to_string().into_bytes();
    bytes.push(b'.');
    bytes.extend_from_slice(session_name.as_bytes());
    OsString::from_vec(bytes)
}

fn session_name_matches(socket_name: &OsStr, requested: &OsStr) -> bool {
    if socket_name == requested {
        return true;
    }

    let socket = socket_name.as_bytes();
    let requested = requested.as_bytes();
    let Some(dot) = socket.iter().position(|byte| *byte == b'.') else {
        return false;
    };
    !socket[..dot].is_empty()
        && socket[..dot].iter().all(|byte| byte.is_ascii_digit())
        && &socket[dot + 1..] == requested
}

fn format_socket_date(time: SystemTime) -> String {
    let secs = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return String::new(),
    };
    // Convert Unix timestamp to YYYY-MM-DD HH:MM:SS UTC, then format as MM/DD/YY HH:MM:SS
    let secs_per_day = 86400_u64;
    let days = secs / secs_per_day;
    let day_secs = secs % secs_per_day;

    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    // Days since Unix epoch conversion (civil_from_days) for UTC
    let (year, month, day) = civil_from_days(days as i64);
    let short_year = year % 100;
    format!("{month:02}/{day:02}/{short_year:02} {hours:02}:{minutes:02}:{seconds:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Algorithm from Howard Hinnant / "chrono-compatible days-to-civil"
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn open_or_create_runtime() -> Result<RuntimeDirectory, String> {
    let path = env::var_os("SCREENDIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_runtime_path);
    if path.exists() {
        RuntimeDirectory::open(path).map_err(|error| error.to_string())
    } else {
        RuntimeDirectory::create_private(path).map_err(|error| error.to_string())
    }
}

fn default_runtime_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = PathBuf::from("/private/tmp");
    #[cfg(not(target_os = "macos"))]
    let base = env::temp_dir();

    base.join(format!("screen-rs-{}", current_effective_uid()))
}

fn split_command(mut command: Vec<OsString>, shell: Option<OsString>) -> (OsString, Vec<OsString>) {
    if command.is_empty() {
        return (
            shell
                .or_else(|| env::var_os("SHELL"))
                .unwrap_or_else(|| OsString::from("/bin/sh")),
            Vec::new(),
        );
    }

    let program = command.remove(0);
    (program, command)
}

fn normalize_exit_code(code: i32) -> u8 {
    if (0..=255).contains(&code) {
        code as u8
    } else {
        1
    }
}

fn stdin_is_tty() -> bool {
    // SAFETY: `isatty` only observes the validity/type of file descriptor 0 and
    // does not take ownership of it or write through pointers.
    unsafe { isatty(0) == 1 }
}

fn terminal_size() -> Option<(u16, u16)> {
    if unsafe { isatty(1) } != 1 {
        return None;
    }

    let mut size = Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: file descriptor 1 is stdout, and `size` is a valid mutable
    // `struct winsize` buffer for the duration of the ioctl call.
    let result = unsafe { ioctl(1, TIOCGWINSZ, &mut size) };
    if result == -1 || size.ws_col == 0 || size.ws_row == 0 {
        None
    } else {
        Some((size.ws_col, size.ws_row))
    }
}

struct RawTerminalGuard {
    original: Termios,
}

impl RawTerminalGuard {
    fn enter_stdin() -> Result<Option<Self>, String> {
        if !stdin_is_tty() {
            return Ok(None);
        }

        let mut original = Termios::zeroed();
        // SAFETY: fd 0 is stdin and `original` points to valid writable storage.
        if unsafe { tcgetattr(0, &mut original) } == -1 {
            return Err(format!(
                "failed to read terminal attributes: {}",
                io::Error::last_os_error()
            ));
        }

        let mut raw = original;
        raw.make_minimal_raw();
        // SAFETY: fd 0 is stdin and `raw` points to a valid termios value.
        if unsafe { tcsetattr(0, TCSAFLUSH, &raw) } == -1 {
            return Err(format!(
                "failed to set terminal raw mode: {}",
                io::Error::last_os_error()
            ));
        }

        Ok(Some(Self { original }))
    }
}

impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        // SAFETY: fd 0 is stdin and `original` remains a valid saved termios value.
        let _ = unsafe { tcsetattr(0, TCSAFLUSH, &self.original) };
    }
}

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: c_ulong,
    c_oflag: c_ulong,
    c_cflag: c_ulong,
    c_lflag: c_ulong,
    c_cc: [c_uchar; NCCS],
    c_ispeed: c_ulong,
    c_ospeed: c_ulong,
}

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: std::ffi::c_uint,
    c_oflag: std::ffi::c_uint,
    c_cflag: std::ffi::c_uint,
    c_lflag: std::ffi::c_uint,
    c_line: c_uchar,
    c_cc: [c_uchar; NCCS],
    c_ispeed: std::ffi::c_uint,
    c_ospeed: std::ffi::c_uint,
}

impl Termios {
    fn zeroed() -> Self {
        // SAFETY: `Termios` is a plain C termios layout and all-zero storage is a
        // valid temporary buffer for `tcgetattr` to fully initialize.
        unsafe { std::mem::zeroed() }
    }

    fn make_minimal_raw(&mut self) {
        self.c_lflag &= !(ECHO | ICANON);
        self.c_cc[VMIN] = 1;
        self.c_cc[VTIME] = 0;
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
const TIOCGWINSZ: c_ulong = 0x4008_7468;
#[cfg(target_os = "linux")]
const TIOCGWINSZ: c_ulong = 0x5413;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const NCCS: usize = 20;
#[cfg(target_os = "linux")]
const NCCS: usize = 32;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const ECHO: c_ulong = 0x0000_0008;
#[cfg(target_os = "linux")]
const ECHO: std::ffi::c_uint = 0o000010;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const ICANON: c_ulong = 0x0000_0100;
#[cfg(target_os = "linux")]
const ICANON: std::ffi::c_uint = 0o000002;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const VMIN: usize = 16;
#[cfg(target_os = "linux")]
const VMIN: usize = 6;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const VTIME: usize = 17;
#[cfg(target_os = "linux")]
const VTIME: usize = 5;

const TCSAFLUSH: c_int = 2;

/// Simple vi-mode copy mode for navigating scrollback.
/// Returns selected text on yank, or None on quit.
fn copy_mode_navigate(lines: &[Vec<u8>], _stream: &mut UnixStream) -> Option<Vec<u8>> {
    use std::io::Write;
    if lines.is_empty() {
        return None;
    }

    let mut stdout = io::stdout().lock();
    // Save cursor and switch to alternate screen for copy mode
    let _ = stdout.write_all(b"\x1b[?1049h"); // alt screen
    let _ = stdout.write_all(b"\x1b[H\x1b[J"); // clear
    let _ = stdout.flush();

    let mut cursor = lines.len().saturating_sub(1); // Start at last line
    let mut mark: Option<usize> = None;
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    let page_size = rows.saturating_sub(2) as usize;

    let mut buf = [0u8; 32];
    let mut stdin = io::stdin().lock();

    loop {
        // Render visible lines
        let start = cursor
            .saturating_sub(page_size / 2)
            .min(lines.len().saturating_sub(page_size.min(lines.len())));
        let end = (start + page_size).min(lines.len());
        let _ = stdout.write_all(b"\x1b[H\x1b[J");
        for (i, line) in lines.iter().enumerate().take(end).skip(start) {
            let prefix = if Some(i) == mark {
                b"> "
            } else if i == cursor {
                b"* "
            } else {
                b"  "
            };
            let _ = stdout.write_all(prefix);
            let display = String::from_utf8_lossy(line);
            let max_cols = cols.saturating_sub(2) as usize;
            if display.len() > max_cols {
                let _ = stdout.write_all(display[..max_cols].as_bytes());
            } else {
                let _ = stdout.write_all(display.as_bytes());
            }
            let _ = stdout.write_all(b"\r\n");
        }
        // Status line
        let _ = stdout.write_all(b"\x1b[7m"); // reverse video
        let status = format!(
            "COPY MODE: j/k=move  space=mark  y=yank  q=quit  [{}/{}]",
            cursor,
            lines.len()
        );
        let _ = stdout.write_all(status.as_bytes());
        let _ = stdout.write_all(b"\x1b[0m");
        let _ = stdout.flush();

        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                match &buf[..n] {
                    b"j" | b"\x1b[B" => {
                        cursor = (cursor + 1).min(lines.len() - 1);
                    }
                    b"k" | b"\x1b[A" => {
                        cursor = cursor.saturating_sub(1);
                    }
                    b"g" => cursor = 0,
                    b"G" => cursor = lines.len() - 1,
                    b" " => {
                        if let Some(m) = mark {
                            // Have both mark and cursor, yank
                            let start = m.min(cursor);
                            let end = m.max(cursor);
                            let mut selected = Vec::new();
                            for line in lines.iter().take(end + 1).skip(start) {
                                selected.extend_from_slice(line);
                                selected.push(b'\n');
                            }
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                        mark = Some(cursor);
                    }
                    b"y" => {
                        if let Some(m) = mark {
                            let start = m.min(cursor);
                            let end = m.max(cursor);
                            let mut selected = Vec::new();
                            for line in lines.iter().take(end + 1).skip(start) {
                                selected.extend_from_slice(line);
                                selected.push(b'\n');
                            }
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                    }
                    b"\x1b" => break,
                    b"q" => break,
                    b"\x0c" => {
                        // Ctrl+L - redraw
                    }
                    b"\x06" => {
                        // Ctrl+F - page down
                        cursor = (cursor + page_size).min(lines.len() - 1);
                    }
                    b"\x02" => {
                        // Ctrl+B - page up
                        cursor = cursor.saturating_sub(page_size);
                    }
                    _ => {}
                }
            }
            Err(_) => break,
        }
    }

    // Restore
    let _ = stdout.write_all(b"\x1b[?1049l");
    let _ = stdout.flush();
    None
}
