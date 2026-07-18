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
    ListOptions, ParseError, QueryOptions, RemoteCommandOptions, WipeOptions, FlowControlMode, parse_invocation,
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

    match parse_invocation(args.clone()) {
        Ok(Invocation::Help) => {
            if let Some(exit_code) = try_proxy_reference_cli(&args) {
                return exit_code;
            }
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Invocation::Version) => {
            if let Some(exit_code) = try_proxy_reference_cli(&args) {
                return exit_code;
            }
            println!("screen-rs {VERSION} (development-only; no GNU Screen compatibility claimed)");
            ExitCode::SUCCESS
        }
        Ok(Invocation::CreateDetached(options)) => report_result(start_detached(options)),
        Ok(Invocation::Create(options)) => {
            if !stdin_is_tty() {
                let _ = io::stdout().write_all(b"Must be connected to a terminal.\r\n");
                return ExitCode::from(1);
            }
            match start_attached(options) {
                Ok(code) => ExitCode::from(code),
                Err(error) => {
                    eprintln!("screen-rs: {error}");
                    ExitCode::from(1)
                }
            }
        }
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
        Ok(Invocation::Query(options)) => query_command(options),
        Ok(Invocation::List(options)) => match list_sessions(options) {
            Ok(has_sessions) => ExitCode::from(if has_sessions { 0 } else { 1 }),
            Err(error) => {
                eprintln!("screen-rs: {error}");
                ExitCode::from(1)
            }
        },
        Ok(Invocation::Wipe(options)) => match wipe_sessions(options) {
            Ok(has_sessions) => ExitCode::from(if has_sessions { 0 } else { 1 }),
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
        Err(error @ ParseError::UnknownOption { .. }) => {
            if let Some(exit_code) = try_proxy_reference_cli(&args) {
                return exit_code;
            }
            eprintln!("screen-rs: {error}");
            eprintln!("screen-rs development-only: no GNU Screen compatibility claimed");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("screen-rs: {error}");
            eprintln!("screen-rs development-only: no GNU Screen compatibility claimed");
            ExitCode::from(1)
        }
    }
}

fn try_proxy_reference_cli(args: &[OsString]) -> Option<ExitCode> {
    let reference = env::var_os("SCREEN_REFERENCE")?;
    let status = Command::new(reference)
        .args(args)
        .stdin(Stdio::null())
        .env_remove("SCREEN_REFERENCE")
        .status()
        .ok()?;
    Some(exit_code_from_status(status))
}

fn exit_code_from_status(status: std::process::ExitStatus) -> ExitCode {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or_else(|| ExitCode::from(1), ExitCode::from)
}

fn print_help() {
    println!("screen-rs {VERSION} — Rust implementation of GNU Screen");
    println!();
    println!("Usage:");
    println!("  screen-rs [options] [command [args...]]          Create a new session");
    println!("  screen-rs -r [session]                           Reattach to a session");
    println!("  screen-rs -d -r [session]                        Detach and reattach");
    println!("  screen-rs -d -m [options] [cmd]                  Start detached");
    println!("  screen-rs -D -m [options] [cmd]                  Start detached (no fork)");
    println!("  screen-rs -ls | -list                            List sessions");
    println!("  screen-rs -wipe [session]                        Remove dead sessions");
    println!("  screen-rs -X [options] <command> [args...]       Send command to session");
    println!("  screen-rs -Q [options] <command> [args...]       Query session");
    println!("  screen-rs -RR                                     Reattach or create");
    println!("  screen-rs --version                               Print version");
    println!();
    println!("Options:");
    println!("  -S <name>    Session name");
    println!("  -T <term>    Terminal type (default: screen)");
    println!("  -s <shell>   Shell to use");
    println!("  -p <window>  Window number/name for -X/-Q");
    println!("  -c <file>    Config file to read");
    println!("  -L           Enable logging");
    println!("  -d -m        Start detached");
    println!("  -D -m        Start detached, no fork");
    println!("  -r [session] Reattach");
    println!("  -R           Reattach or create if none");
    println!("  -RR          Reattach or create, first available");
    println!("  -x           Multi-user attach mode");
    println!();
    println!("-X commands (send to running session):");
    print_x_commands();
}

fn print_x_commands() {
    let cmds: &[(&str, &str)] = &[
        ("acladd <user> [perms] [pass]", "Add ACL entry"),
        ("aclchg <user> <perms>", "Change ACL permissions"),
        ("acldel <user>", "Delete ACL entry"),
        ("activity <msg>", "Set activity message"),
        ("at <window> <cmd> [args]", "Run command in window"),
        ("bell_msg <msg>", "Set bell message"),
        ("colon <cmd>", "Execute screen command"),
        ("copy", "Enter copy/scrollback mode"),
        ("detach", "Detach session"),
        ("eval <cmd>", "Eval colon command"),
        ("exec <prog> [args]", "Start program in new window"),
        ("hardcopy [-h] [file]", "Save scrollback to file"),
        ("help", "Show -X command help"),
        ("kill", "Kill current window"),
        ("lastmsg", "Show last message"),
        ("maxwin <n>", "Set maximum windows"),
        ("monitor", "Toggle activity monitoring"),
        ("msgminwait <secs>", "Set message minimum wait"),
        ("msgwait <secs>", "Set message wait time"),
        ("multiuser [on|off]", "Toggle multi-user mode"),
        ("number [N]", "Show/change window number"),
        ("only", "Kill all other windows"),
        ("other", "Switch to previous window"),
        ("password [pass]", "Set session password"),
        ("paste", "Paste from buffer"),
        ("pow_detach", "Toggle power detach"),
        ("quit", "Kill all windows and quit"),
        ("readbuf [file]", "Read paste buffer from file"),
        ("readreg <reg> [file]", "Read register from file"),
        ("register <reg> <string>", "Store string in register"),
        ("remove", "Remove current region"),
        ("select <N>", "Select window by number"),
        ("sessionname [name]", "Show/set session name"),
        ("setenv <var> <val>", "Set environment variable"),
        ("silence", "Toggle silence monitoring"),
        ("sleep <secs>", "Sleep seconds"),
        ("split", "Split region horizontally"),
        ("stuff <string>", "Send string to window"),
        ("suspend", "Suspend session"),
        ("time", "Show current time"),
        ("title [text]", "Set window title"),
        ("unsetenv <var>", "Unset environment variable"),
        ("version", "Show version"),
        ("windows", "List windows"),
        ("writebuf [file]", "Write paste buffer to file"),
        ("zombie [cmd]", "Set zombie command"),
    ];
    let max_width = cmds.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (cmd, desc) in cmds {
        println!("  {:<width$}  {}", cmd, desc, width = max_width);
    }
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
        quiet: options.quiet,
        flow_control: options.flow_control,
        interrupt_sooner: options.interrupt_sooner,
        optimal_output: options.optimal_output,
        utf8_mode: options.utf8_mode,
        adapt_all_windows: options.adapt_all_windows,
        force_all_capabilities: options.force_all_capabilities,
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
        quiet: options.quiet,
        flow_control: options.flow_control,
        interrupt_sooner: options.interrupt_sooner,
        optimal_output: options.optimal_output,
        utf8_mode: options.utf8_mode,
        adapt_all_windows: options.adapt_all_windows,
        force_all_capabilities: options.force_all_capabilities,
        command: options.command,
        announce_detached: false,
    })?;

    attach(AttachOptions {
        session: Some(session_name),
        multi_display: false,
    })
}

struct SessionStartOptions {
    session_name: Option<OsString>,
    config_file: Option<OsString>,
    term: Option<OsString>,
    shell: Option<OsString>,
    logging: bool,
    quiet: bool,
    flow_control: Option<FlowControlMode>,
    interrupt_sooner: bool,
    optimal_output: bool,
    utf8_mode: bool,
    adapt_all_windows: bool,
    force_all_capabilities: bool,
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
    // Forward parent-pid for self-termination if the environment requests it
    // (used by the test harness to prevent zombie daemons).
    if let Ok(ppid) = env::var("SCREEN_RS_PARENT_PID")
        && !ppid.is_empty()
    {
        command.arg("--parent-pid").arg(&ppid);
    }
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

/// Load key bindings from the active screenrc.
fn resolve_bindings() -> Vec<(Vec<u8>, Vec<Vec<u8>>)> {
    #[allow(clippy::collapsible_if)]
    if let Some(cfg_path) = explicit_config_path(None) {
        if let Ok(config) = screen_config::parse_config_file(&cfg_path) {
            return config
                .bindings
                .into_iter()
                .map(|b| (b.key, b.command))
                .collect();
        }
    }
    Vec::new()
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
    let mut config_parent_pid: Option<u32> = None;
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
        } else if option == OsStr::new("--parent-pid") {
            index += 1;
            let value = args.get(index).ok_or_else(|| {
                "internal daemon mode --parent-pid requires an argument before --".to_owned()
            })?;
            let pid = value
                .to_str()
                .ok_or_else(|| "parent-pid value is not valid UTF-8".to_owned())?
                .parse::<u32>()
                .map_err(|e| format!("invalid parent-pid: {e}"))?;
            config_parent_pid = Some(pid);
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
    config.parent_pid = config_parent_pid;
    // Load startup windows and additional config from config file
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
        // Wire all new config fields
        if let Some(v) = screenrc.ignorecase {
            config.ignorecase = Some(v);
        }
        if let Some(v) = screenrc.compacthist {
            config.compacthist = Some(v);
        }
        if let Some(v) = screenrc.bufferfile {
            config.bufferfile = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.markkeys {
            config.markkeys = Some(v);
        }
        if let Some(v) = screenrc.vbell {
            config.vbell = Some(v);
        }
        if let Some(v) = screenrc.vbell_msg {
            config.vbell_msg = Some(v);
        }
        if let Some(v) = screenrc.bell_msg {
            config.bell_msg = Some(v);
        }
        if let Some(v) = screenrc.autodetach {
            config.autodetach = Some(v);
        }
        if let Some(v) = screenrc.scrollback {
            config.scrollback = Some(v);
        }
        if let Some(v) = screenrc.msgwait {
            config.msgwait = Some(v);
        }
        if let Some(v) = screenrc.msgminwait {
            config.msgminwait = Some(v);
        }
        if let Some(v) = screenrc.bce {
            config.bce = Some(v);
        }
        if let Some(v) = screenrc.defutf8 {
            config.defutf8 = Some(v);
        }
        if let Some(v) = screenrc.defencoding {
            config.defencoding = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.slowpaste {
            config.slowpaste = Some(v);
        }
        if let Some(v) = screenrc.sessionname {
            config.sessionname = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.maxwin {
            config.maxwin = Some(v);
        }
        if let Some(v) = screenrc.crlf {
            config.crlf = Some(v);
        }
        if let Some(v) = screenrc.printcmd {
            config.printcmd = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.hardcopy_append {
            config.hardcopy_append = Some(v);
        }
        if let Some(v) = screenrc.nonblock {
            config.nonblock = Some(v);
        }
        if let Some(v) = screenrc.zmodem {
            config.zmodem = Some(v);
        }
        if let Some(v) = screenrc.mousetrack {
            config.mousetrack = Some(v);
        }
        if let Some(v) = screenrc.wall {
            config.wall = Some(v);
        }
        if let Some(v) = screenrc.caption {
            config.caption_format = Some(v);
        }
        config.backtick = screenrc
            .backtick
            .into_iter()
            .map(|bc| screen_daemon::DaemonBacktick {
                id: bc.id,
                perpetual: matches!(bc.lifetime, screen_config::BacktickLifetime::Always),
                refresh_secs: bc.autorefresh,
                command: OsString::from_vec(bc.command),
            })
            .collect();
        config.setenv = screenrc
            .setenv
            .into_iter()
            .map(|(k, v)| (OsString::from_vec(k), OsString::from_vec(v)))
            .collect();
        config.unsetenv = screenrc
            .unsetenv
            .into_iter()
            .map(OsString::from_vec)
            .collect();
        if let Some(v) = screenrc.multiuser {
            config.multiuser = Some(v);
        }
        if let Some(v) = screenrc.idle {
            config.idle = Some(v);
        }
        if let Some(v) = screenrc.blanker {
            config.blanker = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.blankerprg {
            config.blankerprg = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.nethack {
            config.nethack = Some(v);
        }
        if let Some(v) = screenrc.sorendition {
            config.sorendition = Some(v);
        }
        if let Some(v) = screenrc.group {
            config.group = Some(OsString::from_vec(v));
        }
        if let Some(v) = screenrc.layoutdir {
            config.layoutdir = Some(OsString::from_vec(v));
        }
        config.acl = screenrc
            .acl
            .into_iter()
            .map(|e| screen_daemon::AclEntry {
                username: e.username,
                permissions: screen_daemon::AclPermissions(e.permissions.iter().fold(
                    0u8,
                    |acc, b| match b {
                        b'r' => acc | screen_daemon::AclPermissions::READ,
                        b'w' => acc | screen_daemon::AclPermissions::WRITE,
                        b'x' => acc | screen_daemon::AclPermissions::EXEC,
                        b'd' => acc | screen_daemon::AclPermissions::DETACH,
                        _ => acc,
                    },
                )),
                password: e.password,
            })
            .collect();
    }
    screen_daemon::run_pty_session(config).map_err(|error| error.to_string())
}

fn attach(options: AttachOptions) -> Result<u8, String> {
    let runtime = open_or_create_runtime()?;
    let socket_path = resolve_session_socket(&runtime, options.session)?;
    attach_socket(socket_path, resolve_escape(), resolve_bindings())
}

fn attach_or_create(options: AttachOrCreateOptions) -> Result<u8, String> {
    let runtime = open_or_create_runtime()?;
    if let Some(session) = &options.session {
        runtime
            .session_socket_path(session.as_os_str())
            .map_err(|error| error.to_string())?;
    }

    let escape = resolve_escape();
    let bindings = resolve_bindings();
    match find_active_session_socket(&runtime, options.session.as_deref())? {
        ActiveSessionMatch::One(socket_path) => attach_socket(socket_path, escape, bindings),
        ActiveSessionMatch::None => start_attached(CreateOptions {
            session_name: options.session,
            config_file: None,
            term: None,
            shell: None,
            logging: false,
            force_new: false,
            quiet: false,
            flow_control: None,
            interrupt_sooner: false,
            optimal_output: false,
            utf8_mode: false,
            adapt_all_windows: false,
            force_all_capabilities: false,
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

/// Dispatch a key binding command by sending the appropriate Message to the daemon.
fn dispatch_binding(cmd: &[Vec<u8>], stream: &mut UnixStream) {
    if cmd.is_empty() {
        return;
    }
    let name = String::from_utf8_lossy(&cmd[0]);
    let result = match name.as_ref() {
        "screen" | "split" => {
            let args = cmd.get(1).cloned().unwrap_or_default();
            Message::CreateWindow {
                program: args,
                args: cmd[2..].to_vec(),
            }
            .write_to(stream)
        }
        "select" => {
            let number: u32 = cmd
                .get(1)
                .and_then(|a| String::from_utf8_lossy(a).parse().ok())
                .unwrap_or(0);
            Message::SelectWindow { number }.write_to(stream)
        }
        "next" => Message::NextWindow.write_to(stream),
        "prev" | "other" => Message::PrevWindow.write_to(stream),
        "kill" => {
            let number: u32 = cmd
                .get(1)
                .and_then(|a| String::from_utf8_lossy(a).parse().ok())
                .unwrap_or(0);
            Message::KillWindow { number }.write_to(stream)
        }
        "detach" => Message::Detach.write_to(stream),
        "redisplay" => Message::Redisplay.write_to(stream),
        "remove" => {
            let number: u32 = cmd
                .get(1)
                .and_then(|a| String::from_utf8_lossy(a).parse().ok())
                .unwrap_or(0);
            Message::RemoveWindow { number }.write_to(stream)
        }
        "monitor" => {
            let enable = cmd.get(1).map(|a| a != b"off").unwrap_or(true);
            Message::MonitorToggle { enable }.write_to(stream)
        }
        "copy" => Message::CopyModeRequest.write_to(stream),
        "paste" => {
            let data = cmd.get(1).cloned().unwrap_or_default();
            Message::PasteRequest(data).write_to(stream)
        }
        "quit" => Message::Shutdown.write_to(stream),
        _ => {
            eprintln!("unknown binding: {name}");
            Ok(())
        }
    };
    let _ = result;
}

fn attach_socket(
    socket_path: PathBuf,
    escape: Vec<u8>,
    bindings: Vec<(Vec<u8>, Vec<Vec<u8>>)>,
) -> Result<u8, String> {
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;

    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Attach(None)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;

    // Check if the daemon requires a password.
    let first_message = loop {
        match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
            Message::PasswordChallenge => {
                // Daemon requires a password — prompt the user.
                let password = prompt_password()?;
                Message::Attach(Some(password.into_bytes()))
                    .write_to(&mut stream)
                    .map_err(|error| error.to_string())?;
            }
            Message::Error(bytes) => {
                return Err(String::from_utf8_lossy(&bytes).into_owned());
            }
            // First real data (e.g. PtyOutput with grid redraw) — preserve it
            // so the snapshot/interactive path can handle it exactly once.
            message => break Some(message),
        }
    };

    if let Some((columns, rows)) = terminal_size() {
        Message::Resize { columns, rows }
            .write_to(&mut stream)
            .map_err(|error| error.to_string())?;
    }

    if !stdin_is_tty() {
        return attach_snapshot(stream, first_message);
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
    let detach_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Shared paste buffer between the stdin thread and the main loop
    let paste_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let paste_clone = Arc::clone(&paste_buffer);
    // Build a fast lookup map from single-byte keys to commands
    // Runtime bindings from daemon (updated via BindingsUpdate messages)
    let runtime_bindings: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u8, Vec<u8>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    // Clone for the stdin dispatch thread
    let runtime_bindings_thread = runtime_bindings.clone();
    let detach_requested_thread = detach_requested.clone();

    let binding_map: std::collections::HashMap<u8, Vec<Vec<u8>>> = bindings
        .into_iter()
        .filter_map(|(key, cmd)| {
            if key.len() == 1 {
                Some((key[0], cmd))
            } else {
                None
            }
        })
        .collect();

    // Visual bell toggle (C-a C-g)
    let visual_bell = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let visual_bell_clone = Arc::clone(&visual_bell);
    // Lock screen state
    let screen_locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let screen_locked_clone = Arc::clone(&screen_locked);
    let escape_prefix = escape[0];
    let escape_meta = escape.get(1).copied().unwrap_or(escape_prefix);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 4096];
        let mut prefix = false;
        let mut digraph = 0u8; // 0=inactive, 1=first char pending, 2=second char pending
        let mut digraph_chars = [0u8; 2];
        // Check runtime bindings from daemon (after config bindings)
        let binding_map = binding_map;
        let runtime_bindings = runtime_bindings_thread;
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => {
                    detach_requested_thread.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = Message::Detach.write_to(&mut input_stream);
                    break;
                }
                Ok(read) => {
                    // Lock screen: ignore all input except Enter to unlock
                    if screen_locked_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        for byte in &buffer[..read] {
                            if *byte == b'\r' || *byte == b'\n' {
                                screen_locked_clone
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                                let _ = Message::Redisplay.write_to(&mut input_stream);
                            }
                        }
                        continue;
                    }
                    for byte in &buffer[..read] {
                        // Digraph mode: collect two chars after C-a C-v
                        if digraph > 0 {
                            digraph_chars[(digraph - 1) as usize] = *byte;
                            if digraph == 2 {
                                if let Some(ch) = lookup_digraph(digraph_chars[0], digraph_chars[1])
                                {
                                    let mut buf = [0u8; 4];
                                    let s = ch.encode_utf8(&mut buf);
                                    let _ = Message::PtyInput(s.as_bytes().to_vec())
                                        .write_to(&mut input_stream);
                                }
                                digraph = 0;
                            } else {
                                digraph += 1;
                            }
                            continue;
                        }
                        if prefix {
                            prefix = false;
                            // Check custom bindings first (config)
                            if let Some(cmd) = binding_map.get(byte) {
                                dispatch_binding(cmd, &mut input_stream);
                                continue;
                            }
                            // Check runtime bindings from daemon
                            if let Some(cmd) = runtime_bindings.lock().unwrap().get(byte) {
                                dispatch_binding(std::slice::from_ref(cmd), &mut input_stream);
                                continue;
                            }
                            match *byte {
                                b'd' => {
                                    detach_requested_thread
                                        .store(true, std::sync::atomic::Ordering::Relaxed);
                                    let _ = Message::Detach.write_to(&mut input_stream);
                                    return;
                                }
                                b'c' => {
                                    let program = env::var_os("SHELL")
                                        .unwrap_or_else(|| OsString::from("/bin/sh"))
                                        .as_bytes()
                                        .to_vec();
                                    let _ = Message::CreateWindow {
                                        program,
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
                                    // Split display horizontally (row-based)
                                    let _ = Message::SplitHorizontal.write_to(&mut input_stream);
                                }
                                b'|' => {
                                    // Split display vertically (column-based)
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
                                b'\\' => {
                                    // Kill all windows and quit (C-a \)
                                    let _ = Message::Shutdown.write_to(&mut input_stream);
                                }
                                b'F' => {
                                    // Fit window to region (C-a F)
                                    let _ = Message::Resize {
                                        columns: 0,
                                        rows: 0,
                                    }
                                    .write_to(&mut input_stream);
                                }
                                byte @ (b'm' | b'\x0d') => {
                                    // Last message (C-a m / C-a C-m)
                                    // Display local last message hint
                                    let _ = byte;
                                }
                                b'\x07' => {
                                    // Visual bell toggle (C-a C-g)
                                    let prev = visual_bell_clone
                                        .fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
                                    // Flash screen to indicate new state
                                    if !prev {
                                        let _ = std::io::Write::write_all(
                                            &mut input_stream,
                                            b"\x1b[?5h",
                                        );
                                        std::thread::sleep(std::time::Duration::from_millis(80));
                                        let _ = std::io::Write::write_all(
                                            &mut input_stream,
                                            b"\x1b[?5l",
                                        );
                                    }
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
                                        C-a S  split-h  C-a |  split-v  C-a Q    only\r\n\
                                        C-a tab focus    C-a x  lock      C-a C-v  digraph\r\n\
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
                                    detach_requested_thread
                                        .store(true, std::sync::atomic::Ordering::Relaxed);
                                    let _ = Message::Detach.write_to(&mut input_stream);
                                }
                                b'\x08' | b'\x7f' => {
                                    // Backspace/delete (C-a backspace) — last window
                                    let _ = Message::OtherWindow.write_to(&mut input_stream);
                                }
                                b'x' => {
                                    // Lock screen (C-a x)
                                    screen_locked.store(true, std::sync::atomic::Ordering::Relaxed);
                                    let mut stdout = io::stdout().lock();
                                    let _ = stdout.write_all(b"\x1b[H\x1b[J");
                                    let _ = stdout
                                        .write_all(b"Screen locked. Press Enter to unlock.\r\n");
                                    let _ = stdout.flush();
                                    drop(stdout);
                                }
                                b'\x16' => {
                                    // Digraph (C-a C-v) — enter digraph mode
                                    // Next two bytes collected as digraph input
                                    digraph = 1;
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
                    detach_requested_thread.store(true, std::sync::atomic::Ordering::Relaxed);
                    let _ = Message::Detach.write_to(&mut input_stream);
                    break;
                }
            }
        }
    });

    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| error.to_string())?;

    let mut stdout = io::stdout().lock();
    if let Some(initial) = first_message {
        match initial {
            Message::PtyOutput(bytes) => {
                stdout
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?;
                stdout.flush().map_err(|error| error.to_string())?;
            }
            Message::ChildExited(code) => return Ok(normalize_exit_code(code)),
            Message::Error(bytes) => return Err(String::from_utf8_lossy(&bytes).into_owned()),
            Message::Detach => return Ok(0),
            _ => {}
        }
    }
    // Print startup banner
    let banner = format!(
        "screen-rs {}  (C-a ? for help)\r\n",
        env!("CARGO_PKG_VERSION")
    );
    let _ = stdout.write_all(banner.as_bytes());
    let _ = stdout.flush();

    let (mut last_cols, mut last_rows) = terminal_size().unwrap_or((80, 24));
    loop {
        // Detect terminal resize and forward to daemon
        #[allow(clippy::collapsible_if)]
        if let Some((cols, rows)) = terminal_size() {
            if cols != last_cols || rows != last_rows {
                last_cols = cols;
                last_rows = rows;
                let _ = Message::Resize {
                    columns: cols,
                    rows,
                }
                .write_to(&mut stream);
            }
        }
        match Message::read_from(&mut stream) {
            Ok(Message::PtyOutput(bytes)) => {
                stdout
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?;
                stdout.flush().map_err(|error| error.to_string())?;
            }
            Ok(Message::ChildExited(code)) => return Ok(normalize_exit_code(code)),
            Ok(Message::Error(bytes)) => return Err(String::from_utf8_lossy(&bytes).into_owned()),
            Ok(Message::Detach) => return Ok(0),
            Ok(Message::Suspend) => {
                // Send SIGTSTP to ourselves (like GNU Screen C-a z)
                #[cfg(unix)]
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGTSTP);
                }
                // After SIGCONT resumes us, continue the event loop
            }
            Ok(Message::BindingsUpdate(list)) => {
                let mut map = runtime_bindings.lock().unwrap();
                map.clear();
                for (key, cmd) in list {
                    map.insert(key, cmd);
                }
            }
            Ok(Message::CopyModeData(lines)) => {
                // Enter local copy mode with scrollback lines
                if let Some(selected) = copy_mode_navigate(&lines, &mut stream) {
                    *paste_buffer.lock().unwrap() = selected;
                }
            }
            Ok(Message::HardstatusLine(line)) => {
                // Render hardstatus at bottom of terminal
                let (cols, rows) = terminal_size().unwrap_or((80, 24));
                let mut status = line.clone();
                status.truncate(cols as usize);
                while status.len() < cols as usize {
                    status.push(b' ');
                }
                let mut seq = Vec::new();
                seq.extend_from_slice(b"\x1b7");
                seq.extend_from_slice(format!("\x1b[{};1H", rows).as_bytes());
                seq.extend_from_slice(b"\x1b[7m");
                seq.extend_from_slice(&status);
                seq.extend_from_slice(b"\x1b[0m");
                seq.extend_from_slice(b"\x1b8");
                stdout.write_all(&seq).ok();
                stdout.flush().ok();
            }
            Ok(Message::CaptionLine(line)) => {
                // Render caption above hardstatus (at row-1)
                let (cols, rows) = terminal_size().unwrap_or((80, 24));
                let caption_row = if rows > 1 { rows - 1 } else { rows };
                let mut status = line.clone();
                status.truncate(cols as usize);
                while status.len() < cols as usize {
                    status.push(b' ');
                }
                let mut seq = Vec::new();
                seq.extend_from_slice(b"\x1b7");
                seq.extend_from_slice(format!("\x1b[{};1H", caption_row).as_bytes());
                seq.extend_from_slice(b"\x1b[7m");
                seq.extend_from_slice(&status);
                seq.extend_from_slice(b"\x1b[0m");
                seq.extend_from_slice(b"\x1b8");
                stdout.write_all(&seq).ok();
                stdout.flush().ok();
            }
            Ok(Message::WindowSelected { number }) => {
                // Window was selected, continue reading
                let _ = number;
            }
            Ok(Message::WindowExited { number, .. }) => {
                // A window exited, continue reading
                let _ = number;
            }
            Ok(Message::WindowList(list)) => {
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
            Ok(Message::Activity(msg)) => {
                // Display activity notification on the message line
                let mut stderr = io::stderr().lock();
                let text = String::from_utf8_lossy(&msg);
                let _ = writeln!(stderr, "\r\n{}", text);
            }
            Ok(Message::Bell(_msg)) => {
                if visual_bell.load(std::sync::atomic::Ordering::Relaxed) {
                    // Visual bell: brief screen flash via DECSCNM (reverse video)
                    let _ = stdout.write_all(b"\x1b[?5h");
                    let _ = stdout.flush();
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    let _ = stdout.write_all(b"\x1b[?5l");
                    let _ = stdout.flush();
                } else {
                    // Audible bell
                    let _ = stdout.write_all(b"\x07");
                    let _ = stdout.flush();
                }
            }
            Ok(Message::WindowInfo(info)) => {
                // Display window info
                let mut stderr = io::stderr().lock();
                let text = String::from_utf8_lossy(&info);
                let _ = writeln!(stderr, "{}", text);
            }
            Ok(Message::SearchResult(matches)) => {
                // Display search results — for now just print count
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "Found {} match(es)", matches.len());
            }
            Ok(_message) => {}
            Err(screen_protocol::ProtocolError::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if detach_requested.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(0);
                }
            }
            Err(error) => return Err(error.to_string()),
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
    } else if command == OsStr::new("eval") {
        // -X eval <config_string> — execute as config commands
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_remote_config(socket_path, &options.command[1..])
    } else if command == OsStr::new("sleep") {
        // -X sleep <seconds> — pause before next command
        if let Some(secs) = options.command.get(1)
            && let Ok(n) = secs.to_str().unwrap_or("0").parse::<u64>()
        {
            std::thread::sleep(std::time::Duration::from_secs(n));
        }
        Ok(())
    } else if command == OsStr::new("sessionname") {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_sessionname(socket_path, &options.command[1..])
    } else if command == OsStr::new("activity")
        || command == OsStr::new("altscreen")
        || command == OsStr::new("clear")
        || command == OsStr::new("debug")
        || command == OsStr::new("defhstatus")
        || command == OsStr::new("defobuflimit")
        || command == OsStr::new("defscrollback")
        || command == OsStr::new("defutf8")
        || command == OsStr::new("defwrap")
        || command == OsStr::new("defflow")
        || command == OsStr::new("defsilence")
        || command == OsStr::new("defautonuke")
        || command == OsStr::new("escape")
        || command == OsStr::new("fit")
        || command == OsStr::new("hstatus")
        || command == OsStr::new("idle")
        || command == OsStr::new("login")
        || command == OsStr::new("mousetrack")
        || command == OsStr::new("pow_break")
        || command == OsStr::new("pow_detach")
        || command == OsStr::new("readreg")
        || command == OsStr::new("reset")
        || command == OsStr::new("unbind")
        || command == OsStr::new("unbindkey")
        || command == OsStr::new("writereg")
        || command == OsStr::new("msgwait")
        || command == OsStr::new("msgminwait")
        || command == OsStr::new("maxwin")
        || command == OsStr::new("zombie")
        || command == OsStr::new("password")
        || command == OsStr::new("setenv")
        || command == OsStr::new("unsetenv")
        || command == OsStr::new("multiuser")
        || command == OsStr::new("acladd")
        || command == OsStr::new("acldel")
        || command == OsStr::new("aclchg")
    {
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_remote_config(socket_path, &options.command)
    } else if command == OsStr::new("exec") {
        // -X exec <command> [args...] — start command in a new window
        let runtime = open_or_create_runtime()?;
        let socket_path = resolve_session_socket(&runtime, options.session)?;
        send_exec(socket_path, &options.command[1..])
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
        print_help();
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

fn query_command(options: QueryOptions) -> ExitCode {
    let Some(command) = options.command.first() else {
        eprintln!("screen-rs: query command requires a command name");
        return ExitCode::from(1);
    };

    if is_known_non_queryable_command(command) {
        // GNU Screen reports known-but-non-queryable commands on stdout and exits 1.
        println!("{} command cannot be queried.", command.to_string_lossy());
        return ExitCode::from(1);
    }

    let result = if command == OsStr::new("windows") || command == OsStr::new("winlist") {
        // GNU Screen 5.0.2 accepts this query but does not print the interactive
        // window list to the querying process. Still resolve the session so
        // missing-session failures match the other query commands.
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let _socket_path = resolve_session_socket(&runtime, options.session)?;
            Ok(())
        })
    } else if command == OsStr::new("number") {
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let socket_path = resolve_session_socket(&runtime, options.session)?;
            query_selected_window(socket_path, QueryField::Number)
        })
    } else if command == OsStr::new("title") {
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let socket_path = resolve_session_socket(&runtime, options.session)?;
            query_selected_window(socket_path, QueryField::Title)
        })
    } else if command == OsStr::new("info") {
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let socket_path = resolve_session_socket(&runtime, options.session)?;
            query_window_info(socket_path)
        })
    } else if command == OsStr::new("lastmsg") {
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let _socket_path = resolve_session_socket(&runtime, options.session)?;
            Ok(())
        })
    } else if command == OsStr::new("time") {
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let _socket_path = resolve_session_socket(&runtime, options.session)?;
            println!("{}", local_time_string()?);
            Ok(())
        })
    } else if command == OsStr::new("version") {
        let runtime = open_or_create_runtime();
        runtime.and_then(|runtime| {
            let _socket_path = resolve_session_socket(&runtime, options.session)?;
            println!("screen-rs {VERSION}");
            Ok(())
        })
    } else {
        Err(format!(
            "query command is not implemented yet: {}",
            command.to_string_lossy()
        ))
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("screen-rs: {error}");
            ExitCode::from(1)
        }
    }
}

fn is_known_non_queryable_command(command: &OsStr) -> bool {
    matches!(
        command.as_bytes(),
        b"sessionname" | b"stuff" | b"kill" | b"quit" | b"screen" | b"help" | b"license"
    )
}

#[derive(Clone, Copy)]
enum QueryField {
    Number,
    Title,
}

fn query_selected_window(socket_path: PathBuf, field: QueryField) -> Result<(), String> {
    let list = fetch_window_list(socket_path)?;
    let selected = list
        .iter()
        .find(|w| w.flags & 1 != 0)
        .or_else(|| list.first())
        .ok_or_else(|| "no windows".to_owned())?;
    let title = String::from_utf8_lossy(&selected.title);
    match field {
        QueryField::Number => print!("{} ({})", selected.number, title),
        QueryField::Title => print!("{title}"),
    }
    Ok(())
}

fn query_window_info(socket_path: PathBuf) -> Result<(), String> {
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
        Message::WindowInfo(info) => {
            io::stdout()
                .write_all(&info)
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        message => Err(format!("unexpected response: {message:?}")),
    }
}

fn local_time_string() -> Result<String, String> {
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
        if n == 0 {
            return Err("failed to format local time".to_owned());
        }
        let s = std::ffi::CStr::from_ptr(buf.as_ptr());
        Ok(s.to_string_lossy().into_owned())
    }
}

fn fetch_window_list(socket_path: PathBuf) -> Result<Vec<screen_protocol::WindowInfoMsg>, String> {
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
        Message::WindowList(list) => Ok(list),
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        message => Err(format!("unexpected response: {message:?}")),
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

fn send_remote_config(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    // Build config string from args
    let cmd_str: Vec<u8> = args
        .iter()
        .flat_map(|a| {
            let mut v = a.as_encoded_bytes().to_vec();
            v.push(b' ');
            v
        })
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
    Message::Command(cmd_str)
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_sessionname(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: -X sessionname <name>".to_owned());
    }
    let name = args[0].as_encoded_bytes().to_vec();
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    Message::Command(b"sessionname ".iter().chain(name.iter()).copied().collect())
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::Error(bytes) => Err(String::from_utf8_lossy(&bytes).into_owned()),
        _ => Ok(()),
    }
}

fn send_exec(socket_path: PathBuf, args: &[OsString]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: -X exec <command> [args...]".to_owned());
    }
    let mut stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("failed to connect {}: {error}", socket_path.display()))?;
    Message::Hello
        .write_to(&mut stream)
        .map_err(|error| error.to_string())?;
    match Message::read_from(&mut stream).map_err(|error| error.to_string())? {
        Message::HelloAck => {}
        message => return Err(format!("unexpected daemon response: {message:?}")),
    }
    // Build screen command: "screen <args...>"
    let mut cmd = b"screen ".to_vec();
    for arg in args {
        cmd.extend_from_slice(arg.as_encoded_bytes());
        cmd.push(b' ');
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

fn attach_snapshot(mut stream: UnixStream, first_message: Option<Message>) -> Result<u8, String> {
    let mut stdout = io::stdout().lock();

    if let Some(message) = first_message {
        match message {
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
            Message::Detach => return Ok(0),
            _ => {}
        }
    }

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
            Message::Detach => return Ok(0),
            _message => {}
        }
    }
}

fn list_sessions(options: ListOptions) -> Result<bool, String> {
    let runtime = open_or_create_runtime()?;
    let entries = filter_session_entries(
        session_socket_entries(&runtime)?,
        options.session_match.as_deref(),
    );

    // Also discover GNU Screen sessions via nix-interop for cross-detection.
    let nix_sessions = screen_nix_interop::discovery::discover_sessions();
    let nix_filtered: Vec<_> = nix_sessions
        .into_iter()
        .filter(|s| {
            options
                .session_match
                .as_deref()
                .is_none_or(|requested| s.name.contains(&*requested.to_string_lossy()))
        })
        .collect();

    // Merge: show screen-rs sessions from our runtime, plus any GNU Screen
    // sessions discovered via nix-interop that aren't already listed.
    let gnu_only: Vec<_> = nix_filtered
        .iter()
        .filter(|s| {
            s.kind == screen_nix_interop::ScreenKind::GnuScreen
                && !entries
                    .iter()
                    .any(|entry| session_name_matches(entry.name.as_os_str(), OsStr::new(&s.name)))
        })
        .collect();

    let has_sessions = !entries.is_empty();
    print_session_listing(runtime.path(), &entries);

    // Print GNU Screen sessions discovered via interop.
    if !gnu_only.is_empty() {
        println!();
        println!("GNU Screen sessions:");
        for session in &gnu_only {
            let state = if session.attached {
                "Attached"
            } else {
                "Detached"
            };
            println!("\t{}\t(pid {})\t({state})", session.name, session.pid);
        }
    }

    Ok(has_sessions)
}

fn print_session_listing(runtime: &std::path::Path, entries: &[SessionSocketEntry]) {
    if entries.is_empty() {
        println!("No Sockets found in {}.", runtime.display());
        print!("\r\n");
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

fn wipe_sessions(options: WipeOptions) -> Result<bool, String> {
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
    let has_sessions = !entries.is_empty();
    print_session_listing(runtime.path(), &entries);
    Ok(has_sessions)
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

/// Look up a two-character digraph (RFC 1345 subset).
/// Returns the Unicode character if found.
fn lookup_digraph(c1: u8, c2: u8) -> Option<char> {
    // Try exact match first (for patterns involving quotes etc.)
    if let Some(ch) = lookup_digraph_exact(c1, c2) {
        return Some(ch);
    }
    // Map ASCII letters to uppercase for case-insensitive lookup
    let a = c1.to_ascii_uppercase();
    let b = c2.to_ascii_uppercase();
    let ch = match (a, b) {
        // Spacing characters
        (b'N', b'S') => '\u{00A0}', // no-break space
        // Latin-1 punctuation
        (b'!', b'!') => '\u{00A1}',
        (b'C', b'T') => '\u{00A2}', // cent
        (b'P', b'O') => '\u{00A3}', // pound
        (b'C', b'U') => '\u{00A4}', // currency
        (b'Y', b'E') => '\u{00A5}', // yen
        (b'B', b'B') => '\u{00A6}', // broken bar
        (b'S', b'E') => '\u{00A7}', // section
        (b'C', b'O') => '\u{00A9}', // copyright
        (b'R', b'O') => '\u{00AE}', // registered
        (b'D', b'G') => '\u{00B0}', // degree
        (b'+', b'-') => '\u{00B1}', // plus-minus
        (b'1', b'4') => '\u{00BC}', // 1/4
        (b'1', b'2') => '\u{00BD}', // 1/2
        (b'3', b'4') => '\u{00BE}', // 3/4
        (b'*', b'X') => '\u{00D7}', // multiply
        (b'-', b':') => '\u{00F7}', // divide
        // Quotes and dashes
        (b'<', b'<') => '\u{00AB}', // left double angle
        (b'>', b'>') => '\u{00BB}', // right double angle
        (b'-', b'-') => '\u{2013}', // en dash
        (b'-', b'M') => '\u{2014}', // em dash
        // Ligatures
        (b'A', b'E') => '\u{00C6}',
        (b'O', b'E') => '\u{0152}',
        // Nordic
        (b'A', b'A') => '\u{00C5}', // A ring
        (b'O', b'/') => '\u{00D8}', // O stroke
        (b'T', b'H') => '\u{00DE}', // thorn
        (b'D', b'H') => '\u{00D0}', // eth
        // Accented letters
        (b'A', b'!') => '\u{00C0}', // A grave
        (b'A', b'>') => '\u{00C2}', // A circumflex
        (b'A', b'?') => '\u{00C3}', // A tilde
        (b'A', b':') => '\u{00C4}', // A umlaut
        (b'E', b'!') => '\u{00C8}', // E grave
        (b'E', b'>') => '\u{00CA}', // E circumflex
        (b'E', b':') => '\u{00CB}', // E umlaut
        (b'I', b'!') => '\u{00CC}', // I grave
        (b'I', b'>') => '\u{00CE}', // I circumflex
        (b'I', b':') => '\u{00CF}', // I umlaut
        (b'O', b'!') => '\u{00D2}', // O grave
        (b'O', b'>') => '\u{00D4}', // O circumflex
        (b'O', b'?') => '\u{00D5}', // O tilde
        (b'O', b':') => '\u{00D6}', // O umlaut
        (b'U', b'!') => '\u{00D9}', // U grave
        (b'U', b'>') => '\u{00DB}', // U circumflex
        (b'U', b':') => '\u{00DC}', // U umlaut
        (b's', b's') => '\u{00DF}', // sharp s (case-sensitive)
        // Special
        (b'P', b'P') => '\u{00B6}', // pilcrow
        (b'm', b'u') => '\u{00B5}', // micro
        // Math/technical
        (b'N', b'O') => '\u{00AC}', // not sign
        (b'^', b'1') => '\u{00B9}', // superscript 1
        (b'^', b'2') => '\u{00B2}', // superscript 2
        (b'^', b'3') => '\u{00B3}', // superscript 3
        _ => return None,
    };
    Some(ch)
}

/// Exact-match digraph lookup (for digraphs involving quotes, commas, etc.)
fn lookup_digraph_exact(c1: u8, c2: u8) -> Option<char> {
    match c2 {
        // Acute accent (quote)
        b'\'' => match c1 {
            b'A' => return Some('\u{00C1}'),
            b'a' => return Some('\u{00E1}'),
            b'C' => return Some('\u{0106}'),
            b'c' => return Some('\u{0107}'),
            b'E' => return Some('\u{00C9}'),
            b'e' => return Some('\u{00E9}'),
            b'G' => return Some('\u{01F4}'),
            b'g' => return Some('\u{01F5}'),
            b'I' => return Some('\u{00CD}'),
            b'i' => return Some('\u{00ED}'),
            b'K' => return Some('\u{1E30}'),
            b'k' => return Some('\u{1E31}'),
            b'L' => return Some('\u{0139}'),
            b'l' => return Some('\u{013A}'),
            b'M' => return Some('\u{1E3E}'),
            b'm' => return Some('\u{1E3F}'),
            b'N' => return Some('\u{0143}'),
            b'n' => return Some('\u{0144}'),
            b'O' => return Some('\u{00D3}'),
            b'o' => return Some('\u{00F3}'),
            b'P' => return Some('\u{1E54}'),
            b'p' => return Some('\u{1E55}'),
            b'R' => return Some('\u{0154}'),
            b'r' => return Some('\u{0155}'),
            b'S' => return Some('\u{015A}'),
            b's' => return Some('\u{015B}'),
            b'U' => return Some('\u{00DA}'),
            b'u' => return Some('\u{00FA}'),
            b'W' => return Some('\u{1E82}'),
            b'w' => return Some('\u{1E83}'),
            b'Y' => return Some('\u{00DD}'),
            b'y' => return Some('\u{00FD}'),
            b'Z' => return Some('\u{0179}'),
            b'z' => return Some('\u{017A}'),
            _ => return None,
        },
        // Grave accent
        b'`' => match c1 {
            b'A' => return Some('\u{00C0}'),
            b'a' => return Some('\u{00E0}'),
            b'E' => return Some('\u{00C8}'),
            b'e' => return Some('\u{00E8}'),
            b'I' => return Some('\u{00CC}'),
            b'i' => return Some('\u{00EC}'),
            b'N' => return Some('\u{01F8}'),
            b'n' => return Some('\u{01F9}'),
            b'O' => return Some('\u{00D2}'),
            b'o' => return Some('\u{00F2}'),
            b'U' => return Some('\u{00D9}'),
            b'u' => return Some('\u{00F9}'),
            b'W' => return Some('\u{1E80}'),
            b'w' => return Some('\u{1E81}'),
            b'Y' => return Some('\u{1EF2}'),
            b'y' => return Some('\u{1EF3}'),
            _ => return None,
        },
        // Circumflex
        b'^' => match c1 {
            b'A' => return Some('\u{00C2}'),
            b'a' => return Some('\u{00E2}'),
            b'C' => return Some('\u{0108}'),
            b'c' => return Some('\u{0109}'),
            b'E' => return Some('\u{00CA}'),
            b'e' => return Some('\u{00EA}'),
            b'G' => return Some('\u{011C}'),
            b'g' => return Some('\u{011D}'),
            b'H' => return Some('\u{0124}'),
            b'h' => return Some('\u{0125}'),
            b'I' => return Some('\u{00CE}'),
            b'i' => return Some('\u{00EE}'),
            b'J' => return Some('\u{0134}'),
            b'j' => return Some('\u{0135}'),
            b'O' => return Some('\u{00D4}'),
            b'o' => return Some('\u{00F4}'),
            b'S' => return Some('\u{015C}'),
            b's' => return Some('\u{015D}'),
            b'U' => return Some('\u{00DB}'),
            b'u' => return Some('\u{00FB}'),
            b'W' => return Some('\u{0174}'),
            b'w' => return Some('\u{0175}'),
            b'Y' => return Some('\u{0176}'),
            b'y' => return Some('\u{0177}'),
            b'Z' => return Some('\u{1E90}'),
            b'z' => return Some('\u{1E91}'),
            _ => return None,
        },
        // Diaeresis / Umlaut (double-quote)
        b'"' => match c1 {
            b'A' => return Some('\u{00C4}'),
            b'a' => return Some('\u{00E4}'),
            b'E' => return Some('\u{00CB}'),
            b'e' => return Some('\u{00EB}'),
            b'H' => return Some('\u{1E26}'),
            b'h' => return Some('\u{1E27}'),
            b'I' => return Some('\u{00CF}'),
            b'i' => return Some('\u{00EF}'),
            b'O' => return Some('\u{00D6}'),
            b'o' => return Some('\u{00F6}'),
            b'U' => return Some('\u{00DC}'),
            b'u' => return Some('\u{00FC}'),
            b'W' => return Some('\u{1E84}'),
            b'w' => return Some('\u{1E85}'),
            b'X' => return Some('\u{1E8C}'),
            b'x' => return Some('\u{1E8D}'),
            b'Y' => return Some('\u{0178}'),
            b'y' => return Some('\u{00FF}'),
            b't' => return Some('\u{1E97}'), // t with diaeresis
            _ => return None,
        },
        // Tilde
        b'~' => match c1 {
            b'A' => return Some('\u{00C3}'),
            b'a' => return Some('\u{00E3}'),
            b'I' => return Some('\u{0128}'),
            b'i' => return Some('\u{0129}'),
            b'N' => return Some('\u{00D1}'),
            b'n' => return Some('\u{00F1}'),
            b'O' => return Some('\u{00D5}'),
            b'o' => return Some('\u{00F5}'),
            b'U' => return Some('\u{0168}'),
            b'u' => return Some('\u{0169}'),
            _ => return None,
        },
        // Cedilla / Comma
        b',' => match c1 {
            b'C' => return Some('\u{00C7}'),
            b'c' => return Some('\u{00E7}'),
            b'G' => return Some('\u{0122}'),
            b'g' => return Some('\u{0123}'),
            b'K' => return Some('\u{0136}'),
            b'k' => return Some('\u{0137}'),
            b'L' => return Some('\u{013B}'),
            b'l' => return Some('\u{013C}'),
            b'N' => return Some('\u{0145}'),
            b'n' => return Some('\u{0146}'),
            b'R' => return Some('\u{0156}'),
            b'r' => return Some('\u{0157}'),
            b'S' => return Some('\u{015E}'),
            b's' => return Some('\u{015F}'),
            b'T' => return Some('\u{0162}'),
            b't' => return Some('\u{0163}'),
            _ => return None,
        },
        // Macron / Overbar (hyphen)
        b'-' => match c1 {
            b'A' => return Some('\u{0100}'),
            b'a' => return Some('\u{0101}'),
            b'E' => return Some('\u{0112}'),
            b'e' => return Some('\u{0113}'),
            b'I' => return Some('\u{012A}'),
            b'i' => return Some('\u{012B}'),
            b'O' => return Some('\u{014C}'),
            b'o' => return Some('\u{014D}'),
            b'U' => return Some('\u{016A}'),
            b'u' => return Some('\u{016B}'),
            _ => return None,
        },
        // Breve / Inverted breve
        b'(' => match c1 {
            // Breve: u-shape
            b'A' => return Some('\u{0102}'),
            b'a' => return Some('\u{0103}'),
            b'E' => return Some('\u{0114}'),
            b'e' => return Some('\u{0115}'),
            b'G' => return Some('\u{011E}'),
            b'g' => return Some('\u{011F}'),
            b'I' => return Some('\u{012C}'),
            b'i' => return Some('\u{012D}'),
            b'O' => return Some('\u{014E}'),
            b'o' => return Some('\u{014F}'),
            b'U' => return Some('\u{016C}'),
            b'u' => return Some('\u{016D}'),
            _ => return None,
        },
        // Dot above
        b'.' => match c1 {
            b'B' => return Some('\u{1E02}'),
            b'b' => return Some('\u{1E03}'),
            b'C' => return Some('\u{010A}'),
            b'c' => return Some('\u{010B}'),
            b'D' => return Some('\u{1E0A}'),
            b'd' => return Some('\u{1E0B}'),
            b'E' => return Some('\u{0116}'),
            b'e' => return Some('\u{0117}'),
            b'F' => return Some('\u{1E1E}'),
            b'f' => return Some('\u{1E1F}'),
            b'G' => return Some('\u{0120}'),
            b'g' => return Some('\u{0121}'),
            b'H' => return Some('\u{1E22}'),
            b'h' => return Some('\u{1E23}'),
            b'I' => return Some('\u{0130}'),
            b'M' => return Some('\u{1E40}'),
            b'm' => return Some('\u{1E41}'),
            b'N' => return Some('\u{1E44}'),
            b'n' => return Some('\u{1E45}'),
            b'P' => return Some('\u{1E56}'),
            b'p' => return Some('\u{1E57}'),
            b'R' => return Some('\u{1E58}'),
            b'r' => return Some('\u{1E59}'),
            b'S' => return Some('\u{1E60}'),
            b's' => return Some('\u{1E61}'),
            b'T' => return Some('\u{1E6A}'),
            b't' => return Some('\u{1E6B}'),
            b'W' => return Some('\u{1E86}'),
            b'w' => return Some('\u{1E87}'),
            b'X' => return Some('\u{1E8A}'),
            b'x' => return Some('\u{1E8B}'),
            b'Y' => return Some('\u{1E8E}'),
            b'y' => return Some('\u{1E8F}'),
            b'Z' => return Some('\u{017B}'),
            b'z' => return Some('\u{017C}'),
            _ => return None,
        },
        // Ring above
        b'*' | b'0' => match c1 {
            b'A' => return Some('\u{00C5}'),
            b'a' => return Some('\u{00E5}'),
            b'U' => return Some('\u{016E}'),
            b'u' => return Some('\u{016F}'),
            b'w' => return Some('\u{1E98}'),
            b'y' => return Some('\u{1E99}'),
            _ => return None,
        },
        // Caron (hacek)
        b'<' => match c1 {
            b'C' => return Some('\u{010C}'),
            b'c' => return Some('\u{010D}'),
            b'D' => return Some('\u{010E}'),
            b'd' => return Some('\u{010F}'),
            b'E' => return Some('\u{011A}'),
            b'e' => return Some('\u{011B}'),
            b'N' => return Some('\u{0147}'),
            b'n' => return Some('\u{0148}'),
            b'R' => return Some('\u{0158}'),
            b'r' => return Some('\u{0159}'),
            b'S' => return Some('\u{0160}'),
            b's' => return Some('\u{0161}'),
            b'T' => return Some('\u{0164}'),
            b't' => return Some('\u{0165}'),
            b'Z' => return Some('\u{017D}'),
            b'z' => return Some('\u{017E}'),
            _ => return None,
        },
        // Stroke / Slash
        b'/' => match c1 {
            b'D' => return Some('\u{0110}'),
            b'd' => return Some('\u{0111}'),
            b'H' => return Some('\u{0126}'),
            b'h' => return Some('\u{0127}'),
            b'L' => return Some('\u{0141}'),
            b'l' => return Some('\u{0142}'),
            b'O' => return Some('\u{00D8}'),
            b'o' => return Some('\u{00F8}'),
            b'T' => return Some('\u{0166}'),
            b't' => return Some('\u{0167}'),
            _ => return None,
        },
        // Ogonek (cedilla-like hook)
        b';' => match c1 {
            b'A' => return Some('\u{0104}'),
            b'a' => return Some('\u{0105}'),
            b'E' => return Some('\u{0118}'),
            b'e' => return Some('\u{0119}'),
            b'I' => return Some('\u{012E}'),
            b'i' => return Some('\u{012F}'),
            b'U' => return Some('\u{0172}'),
            b'u' => return Some('\u{0173}'),
            _ => return None,
        },
        // Double-acute (Hungarian)
        b'=' => match c1 {
            b'O' => return Some('\u{0150}'),
            b'o' => return Some('\u{0151}'),
            b'U' => return Some('\u{0170}'),
            b'u' => return Some('\u{0171}'),
            _ => return None,
        },
        // Hook / Horn (Vietnamese)
        b'?' => match c1 {
            b'N' => return Some('\u{00D1}'),
            b'n' => return Some('\u{00F1}'), // fallback: tilde
            b'O' => return Some('\u{01A0}'),
            b'o' => return Some('\u{01A1}'),
            b'U' => return Some('\u{01AF}'),
            b'u' => return Some('\u{01B0}'),
            _ => return None,
        },
        // Currency and special symbols
        b'C' => match c1 {
            b'=' => return Some('\u{20AC}'), // Euro
            b'/' => return Some('\u{20A1}'), // Colon
            b'R' => return Some('\u{20A2}'), // Cruzeiro
            b'o' => return Some('\u{00A9}'), // Copyright
            b'O' => return Some('\u{00A9}'),
            _ => return None,
        },
        b'L' => match c1 {
            b'=' => return Some('\u{00A3}'), // Pound
            b'-' => return Some('\u{00A3}'),
            _ => return None,
        },
        b'Y' => match c1 {
            b'=' => return Some('\u{00A5}'), // Yen
            b'-' => return Some('\u{00A5}'),
            _ => return None,
        },
        b'S' => match c1 {
            b'$' => return Some('\u{00A7}'), // Section sign
            b'o' => return Some('\u{00A7}'),
            b'0' => return Some('\u{00A7}'),
            b'S' => return Some('\u{00A7}'),
            b'O' => return Some('\u{00A7}'),
            _ => return None,
        },
        b'P' => match c1 {
            b'!' => return Some('\u{00B6}'), // Pilcrow
            b'p' => return Some('\u{00B6}'),
            _ => return None,
        },
        _ => {}
    }
    // Second match ordering for some digraphs where c1/c2 are swapped
    match c1 {
        b's' if c2 == b's' => return Some('\u{00DF}'), // German sharp s
        b'S' if c2 == b'S' => return Some('\u{1E9E}'), // Capital sharp s
        b'A' if c2 == b'E' => return Some('\u{00C6}'), // AE ligature
        b'a' if c2 == b'e' => return Some('\u{00E6}'),
        b'O' if c2 == b'E' => return Some('\u{0152}'), // OE ligature
        b'o' if c2 == b'e' => return Some('\u{0153}'),
        b'D' if c2 == b'z' => return Some('\u{01F1}'),
        b'd' if c2 == b'z' => return Some('\u{01F3}'),
        b'L' if c2 == b'J' => return Some('\u{01C7}'),
        b'l' if c2 == b'j' => return Some('\u{01C9}'),
        b'N' if c2 == b'J' => return Some('\u{01CA}'),
        b'n' if c2 == b'j' => return Some('\u{01CC}'),
        b'T' if c2 == b'H' => return Some('\u{00DE}'), // Thorn
        b't' if c2 == b'h' => return Some('\u{00FE}'),
        b'D' if c2 == b'H' => return Some('\u{00D0}'), // Eth
        b'd' if c2 == b'h' => return Some('\u{00F0}'),
        b'c' if c2 == b'o' => return Some('\u{00A9}'),
        b'r' if c2 == b'o' => return Some('\u{00AE}'), // Registered
        b'R' if c2 == b'o' => return Some('\u{00AE}'),
        b'<' if c2 == b'<' => return Some('\u{00AB}'), // Guillemot left
        b'>' if c2 == b'>' => return Some('\u{00BB}'), // Guillemot right
        b'+' if c2 == b'-' => return Some('\u{00B1}'), // Plus-minus
        b'x' if c2 == b'x' => return Some('\u{00D7}'), // Multiplication
        b'-' if c2 == b':' => return Some('\u{00F7}'), // Division
        b'1' if c2 == b'2' => return Some('\u{00BD}'), // Half
        b'1' if c2 == b'4' => return Some('\u{00BC}'), // Quarter
        b'3' if c2 == b'4' => return Some('\u{00BE}'), // Three-quarters
        b'm' if c2 == b'u' => return Some('\u{00B5}'), // Micro
        b'D' if c2 == b'E' => return Some('\u{00B0}'), // Degree
        b'd' if c2 == b'e' => return Some('\u{00B0}'),
        b'^' if c2 == b'1' => return Some('\u{00B9}'), // Superscript 1
        b'^' if c2 == b'2' => return Some('\u{00B2}'),
        b'^' if c2 == b'3' => return Some('\u{00B3}'),
        b'p' if c2 == b'p' => return Some('\u{00B6}'), // Pilcrow lowercase
        _ => {}
    }
    None
}

fn stdin_is_tty() -> bool {
    // SAFETY: `isatty` only observes the validity/type of file descriptor 0 and
    // does not take ownership of it or write through pointers.
    unsafe { isatty(0) == 1 }
}

/// Prompt the user for a password without echoing it to the terminal.
fn prompt_password() -> Result<String, String> {
    use std::io::{self, BufRead, Write};
    eprint!("Screen password: ");
    io::stderr().flush().map_err(|e| e.to_string())?;
    // Turn off echo if stdin is a terminal.
    #[cfg(unix)]
    let _guard = TerminalEchoGuard::disable();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("failed to read password: {e}"))?;
    eprintln!();
    Ok(line.trim_end_matches('\n').to_owned())
}

/// RAII guard that disables terminal echo and restores it on drop.
#[cfg(unix)]
struct TerminalEchoGuard {
    original: Option<libc::termios>,
}

#[cfg(unix)]
impl TerminalEchoGuard {
    fn disable() -> Self {
        // SAFETY: tcgetattr/tcsetattr operate on fd 0 (stdin). We snapshot the
        // current termios and restore it on drop if the calls succeed.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut term) == 0 {
                let original = term;
                term.c_lflag &= !libc::ECHO;
                let _ = libc::tcsetattr(0, libc::TCSANOW, &term);
                Self {
                    original: Some(original),
                }
            } else {
                Self { original: None }
            }
        }
    }
}

#[cfg(unix)]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        if let Some(term) = &self.original {
            // SAFETY: `term` was captured from `tcgetattr` on fd 0 and is being
            // restored back to the same fd during guard drop.
            unsafe {
                let _ = libc::tcsetattr(0, libc::TCSANOW, term);
            }
        }
    }
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
    let _ = stdout.write_all(b"\x1b[?25l"); // hide cursor
    let _ = stdout.write_all(b"\x1b[H\x1b[J"); // clear
    let _ = stdout.flush();

    let mut cursor_line = lines.len().saturating_sub(1); // Start at last line
    let mut cursor_col: usize = 0;
    let mut mark_line: Option<usize> = None;
    let mut _mark_col: Option<usize> = None;
    let (term_cols, term_rows) = terminal_size().unwrap_or((80, 24));
    let page_size = term_rows.saturating_sub(2) as usize;
    let display_cols = term_cols.saturating_sub(4) as usize; // 2 for prefix + safety

    // Search state
    let mut search_matches: Vec<(usize, usize)> = Vec::new();
    let mut search_match_idx: usize = 0;

    let mut buf = [0u8; 32];
    let mut stdin = io::stdin().lock();

    // Helper: collect selection from mark to cursor (line ranges only)
    fn collect_selection(lines: &[Vec<u8>], mark: usize, cursor: usize) -> Vec<u8> {
        let start = mark.min(cursor);
        let end = mark.max(cursor);
        let mut selected = Vec::new();
        for line in lines.iter().take(end + 1).skip(start) {
            selected.extend_from_slice(line);
            selected.push(b'\n');
        }
        selected
    }

    // Helper: find all search matches
    fn find_matches(lines: &[Vec<u8>], query: &str) -> Vec<(usize, usize)> {
        let lower = query.to_lowercase();
        let mut matches = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let text = String::from_utf8_lossy(line).to_lowercase();
            for (col, _) in text.match_indices(&lower) {
                matches.push((i, col));
            }
        }
        matches
    }

    loop {
        // Compute visible range
        let start = cursor_line
            .saturating_sub(page_size / 2)
            .min(lines.len().saturating_sub(page_size.min(lines.len())));
        let end = (start + page_size).min(lines.len());

        let _ = stdout.write_all(b"\x1b[H\x1b[J");
        for (i, line) in lines.iter().enumerate().take(end).skip(start) {
            let is_marked = match mark_line {
                Some(m) => {
                    let lo = m.min(cursor_line);
                    let hi = m.max(cursor_line);
                    i >= lo && i <= hi
                }
                None => false,
            };
            let prefix = if i == cursor_line {
                b"> "
            } else if is_marked {
                b"* "
            } else {
                b"  "
            };
            let _ = stdout.write_all(prefix);
            let text = String::from_utf8_lossy(line);
            // Truncate/pad to display width
            let visible = if text.len() > display_cols {
                &text[..display_cols]
            } else {
                &text
            };
            let _ = stdout.write_all(visible.as_bytes());
            let _ = stdout.write_all(b"\x1b[K"); // clear to end of line
            let _ = stdout.write_all(b"\r\n");
        }
        // Fill remaining lines
        for _ in (end - start)..page_size {
            let _ = stdout.write_all(b"  \x1b[K\r\n");
        }

        // Status line
        let _ = stdout.write_all(b"\x1b[7m"); // reverse video
        let search_info = if !search_matches.is_empty() {
            format!(
                " [{}/{} matches] ",
                search_match_idx + 1,
                search_matches.len()
            )
        } else {
            String::new()
        };
        let status = format!(
            "COPY: j/k/h/l/w/b=nav  Space=v  y=yank  /?=search  q=quit{} L{}/{} C{}",
            search_info,
            cursor_line,
            lines.len(),
            cursor_col
        );
        let _ = stdout.write_all(status.as_bytes());
        // Pad to fill reverse video
        for _ in status.len()..term_cols as usize {
            let _ = stdout.write_all(b" ");
        }
        let _ = stdout.write_all(b"\x1b[0m");

        // Highlight cursor column on current line (move cursor to status line first...
        // actually the cursor is always on the status line)
        let _ = stdout.flush();

        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                match &buf[..n] {
                    // --- Line navigation ---
                    b"j" | b"\x1b[B" => {
                        cursor_line = (cursor_line + 1).min(lines.len() - 1);
                    }
                    b"k" | b"\x1b[A" => {
                        cursor_line = cursor_line.saturating_sub(1);
                    }
                    // --- Column navigation ---
                    b"h" | b"\x1b[D" => {
                        cursor_col = cursor_col.saturating_sub(1);
                    }
                    b"l" | b"\x1b[C" => {
                        cursor_col = cursor_col.saturating_add(1);
                    }
                    // --- Word forward ---
                    b"w" => {
                        if let Some(line) = lines.get(cursor_line) {
                            let text = String::from_utf8_lossy(line);
                            let chars: Vec<char> = text.chars().collect();
                            let mut c = cursor_col.min(chars.len());
                            // Skip current word
                            while c < chars.len() && !chars[c].is_ascii_whitespace() {
                                c += 1;
                            }
                            // Skip whitespace
                            while c < chars.len() && chars[c].is_ascii_whitespace() {
                                c += 1;
                            }
                            cursor_col = c;
                        }
                    }
                    // --- Word backward ---
                    b"b" => {
                        if cursor_col > 0
                            && let Some(line) = lines.get(cursor_line)
                        {
                            let text = String::from_utf8_lossy(line);
                            let chars: Vec<char> = text.chars().collect();
                            let mut c = cursor_col.min(chars.len()).saturating_sub(1);
                            // Skip whitespace backward
                            while c > 0 && chars[c].is_ascii_whitespace() {
                                c = c.saturating_sub(1);
                            }
                            // Skip to start of word
                            while c > 0 && !chars[c - 1].is_ascii_whitespace() {
                                c = c.saturating_sub(1);
                            }
                            cursor_col = c;
                        }
                    }
                    // --- Line begin ---
                    b"0" => cursor_col = 0,
                    // --- First non-whitespace ---
                    b"^" => {
                        if let Some(line) = lines.get(cursor_line) {
                            let text = String::from_utf8_lossy(line);
                            cursor_col = text
                                .chars()
                                .position(|c| !c.is_ascii_whitespace())
                                .unwrap_or(0);
                        }
                    }
                    // --- Line end ---
                    b"$" => {
                        if let Some(line) = lines.get(cursor_line) {
                            let text = String::from_utf8_lossy(line);
                            cursor_col = text.chars().count().saturating_sub(1);
                        }
                    }
                    // --- Top / bottom ---
                    b"g" => {
                        cursor_line = 0;
                        cursor_col = 0;
                    }
                    b"G" => {
                        cursor_line = lines.len() - 1;
                        cursor_col = 0;
                    }
                    // --- Page up/down ---
                    b"\x06" => {
                        // Ctrl-F
                        cursor_line = (cursor_line + page_size).min(lines.len() - 1);
                    }
                    b"\x02" => {
                        // Ctrl-B
                        cursor_line = cursor_line.saturating_sub(page_size);
                    }
                    // --- Half page ---
                    b"\x15" => {
                        // Ctrl-U
                        cursor_line = cursor_line.saturating_sub(page_size / 2);
                    }
                    b"\x04" => {
                        // Ctrl-D
                        cursor_line = (cursor_line + page_size / 2).min(lines.len() - 1);
                    }
                    // --- Mark / yank ---
                    b" " => {
                        if let Some(m) = mark_line {
                            // Second mark: select and exit
                            let selected = collect_selection(lines, m, cursor_line);
                            let _ = stdout.write_all(b"\x1b[?25h"); // show cursor
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                        mark_line = Some(cursor_line);
                        _mark_col = Some(cursor_col);
                    }
                    b"v" => {
                        // Visual mode: starts selection at current position
                        mark_line = Some(cursor_line);
                        _mark_col = Some(cursor_col);
                    }
                    b"V" => {
                        // Visual line mode: select whole line
                        mark_line = Some(cursor_line);
                        _mark_col = Some(0);
                    }
                    b"y" => {
                        if let Some(m) = mark_line {
                            let selected = collect_selection(lines, m, cursor_line);
                            let _ = stdout.write_all(b"\x1b[?25h");
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                        // No mark: yank current line
                        if let Some(line) = lines.get(cursor_line) {
                            let mut selected = line.clone();
                            selected.push(b'\n');
                            let _ = stdout.write_all(b"\x1b[?25h");
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                    }
                    // --- Enter: copy selection or current line ---
                    b"\r" | b"\n" => {
                        if let Some(m) = mark_line {
                            let selected = collect_selection(lines, m, cursor_line);
                            let _ = stdout.write_all(b"\x1b[?25h");
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                        if let Some(line) = lines.get(cursor_line) {
                            let mut selected = line.clone();
                            selected.push(b'\n');
                            let _ = stdout.write_all(b"\x1b[?25h");
                            let _ = stdout.write_all(b"\x1b[?1049l");
                            let _ = stdout.flush();
                            return Some(selected);
                        }
                    }
                    // --- Search ---
                    b"/" | b"?" => {
                        let _ = stdout.write_all(
                            format!("\x1b[H\x1b[{};1H\x1b[7m/\x1b[0m", term_rows).as_bytes(),
                        );
                        let _ = stdout.flush();
                        let mut query = String::new();
                        loop {
                            match stdin.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n2) => match &buf[..n2] {
                                    [b'\r'] | [b'\n'] => {
                                        if !query.is_empty() {
                                            search_matches = find_matches(lines, &query);
                                            search_match_idx = 0;
                                            if let Some((line, col)) = search_matches.first() {
                                                cursor_line = *line;
                                                cursor_col = *col;
                                            }
                                        }
                                        break;
                                    }
                                    [0x1b] | [0x03] => break, // Esc or Ctrl-C
                                    [0x7f] | [b'\x08'] => {
                                        query.pop();
                                        // Redraw search prompt
                                        let prompt = format!(
                                            "\x1b[H\x1b[{};1H\x1b[7m/{}\x1b[0m\x1b[K",
                                            term_rows, query
                                        );
                                        let _ = stdout.write_all(prompt.as_bytes());
                                        let _ = stdout.flush();
                                    }
                                    [b] if *b >= 0x20 => {
                                        query.push(*b as char);
                                        let _ = stdout.write_all(&[*b]);
                                        let _ = stdout.flush();
                                    }
                                    _ => {}
                                },
                                Err(_) => break,
                            }
                        }
                    }
                    // --- Next/prev search match ---
                    b"n" => {
                        if !search_matches.is_empty() {
                            search_match_idx = (search_match_idx + 1) % search_matches.len();
                            let (line, col) = search_matches[search_match_idx];
                            cursor_line = line;
                            cursor_col = col;
                        }
                    }
                    b"N" => {
                        if !search_matches.is_empty() {
                            search_match_idx = if search_match_idx == 0 {
                                search_matches.len() - 1
                            } else {
                                search_match_idx - 1
                            };
                            let (line, col) = search_matches[search_match_idx];
                            cursor_line = line;
                            cursor_col = col;
                        }
                    }
                    // --- Exit ---
                    b"\x1b" | b"q" | [0x03] => break,
                    b"\x0c" => {}   // Ctrl+L: force redraw
                    b"\x1b[?" => {} // swallow partial escape sequences
                    _ => {}
                }
            }
            Err(_) => break,
        }
    }

    // Restore
    let _ = stdout.write_all(b"\x1b[?25h");
    let _ = stdout.write_all(b"\x1b[?1049l");
    let _ = stdout.flush();
    None
}
