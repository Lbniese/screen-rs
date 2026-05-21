#![forbid(unsafe_code)]

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

const SOURCE_RECURSION_LIMIT: usize = 16;

// ---------------------------------------------------------------------------
// Configuration model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandContext {
    Startup,
    Runtime,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScreenConfig {
    pub shell: Option<Vec<u8>>,
    pub term: Option<Vec<u8>>,
    pub chdir: Option<Vec<u8>>,
    pub logging: Option<bool>,
    pub logfile: Option<Vec<u8>>,
    pub escape: Option<Vec<u8>>,
    pub startup_message: Option<bool>,
    pub defscrollback: Option<u32>,
    pub defmonitor: Option<bool>,
    pub defflow: Option<bool>,
    pub defwrap: Option<bool>,
    pub defsilence: Option<u16>,
    pub defautonuke: Option<bool>,
    pub defzombie: Option<Vec<u8>>,
    pub hardstatus: Option<Vec<u8>>,
    pub caption: Option<Vec<u8>>,
    pub bindings: Vec<KeyBinding>,
    pub startup_windows: Vec<StartupWindow>,
    pub select: Option<u32>,
    /// Search case sensitivity.
    pub ignorecase: Option<bool>,
    /// Compact empty lines in scrollback.
    pub compacthist: Option<bool>,
    /// File for exchange buffer (readbuf/writebuf).
    pub bufferfile: Option<Vec<u8>>,
    /// Key sequences for copy mode mark operations.
    pub markkeys: Option<Vec<u8>>,
    /// Visual bell support (vbell on/off).
    pub vbell: Option<bool>,
    /// Visual bell message.
    pub vbell_msg: Option<Vec<u8>>,
    /// Audible bell message.
    pub bell_msg: Option<Vec<u8>>,
    /// Auto-detach on hangup.
    pub autodetach: Option<bool>,
    /// Per-window scrollback buffer size.
    pub scrollback: Option<u32>,
    /// Message display time in seconds.
    pub msgwait: Option<u32>,
    /// Minimum message wait time.
    pub msgminwait: Option<u32>,
    /// Background color erase.
    pub bce: Option<bool>,
    /// Default UTF-8 mode for new windows.
    pub defutf8: Option<bool>,
    /// Default character encoding for new windows.
    pub defencoding: Option<Vec<u8>>,
    /// Slow paste delay in ms.
    pub slowpaste: Option<u32>,
    /// Session name for reattach.
    pub sessionname: Option<Vec<u8>>,
    /// Session password.
    pub password: Option<Vec<u8>>,
    /// Maximum number of windows.
    pub maxwin: Option<u32>,
    /// Maximum scrollback lines (alias for scrollback).
    pub defhistsize: Option<u32>,
    /// CR/LF behavior.
    pub crlf: Option<bool>,
    /// Command to run for hardcopy (e.g., lpr).
    pub printcmd: Option<Vec<u8>>,
    /// Whether hardcopy appends or overwrites.
    pub hardcopy_append: Option<bool>,
    /// Non-blocking I/O mode.
    pub nonblock: Option<bool>,
    /// Zmodem catch support.
    pub zmodem: Option<bool>,
    /// Wall message broadcast.
    pub wall: Option<Vec<u8>>,
    /// Bootstrap backtick commands.
    pub backtick: Vec<BacktickCommand>,
    /// Set environment variables at session start.
    pub setenv: Vec<(Vec<u8>, Vec<u8>)>,
    /// Unset environment variables at session start.
    pub unsetenv: Vec<Vec<u8>>,
    /// Multi-user mode.
    pub multiuser: Option<bool>,
    /// ACL entries.
    pub acl: Vec<ConfigAclEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: Vec<u8>,
    pub command: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartupWindow {
    pub title: Option<Vec<u8>>,
    pub program: Option<Vec<u8>>,
    pub args: Vec<Vec<u8>>,
    pub number: Option<u32>,
    /// Working directory for this window.
    pub working_directory: Option<Vec<u8>>,
    /// Initial bytes to stuff into the window.
    pub stuff: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktickCommand {
    pub id: u16,
    pub lifetime: BacktickLifetime,
    pub autorefresh: Option<u32>,
    pub command: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigAclEntry {
    pub username: Vec<u8>,
    pub permissions: Vec<u8>,
    pub password: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BacktickLifetime {
    Once,
    Always,
}

// ---------------------------------------------------------------------------
// Command specification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub context: CommandContext,
    pub min_args: usize,
    pub max_args: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn parse_config(bytes: &[u8]) -> Result<ScreenConfig, ConfigError> {
    parse_config_with_sources(bytes, &mut |_source, _line| Ok(None))
}

pub fn parse_config_file(path: impl AsRef<Path>) -> Result<ScreenConfig, ConfigError> {
    parse_config_file_inner(path.as_ref(), 0)
}

pub fn default_config_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".screenrc"))
}

/// Parse config file with an explicit path or default discovery.
pub fn load_config(path: Option<&Path>) -> Result<ScreenConfig, ConfigError> {
    match path {
        Some(p) => parse_config_file(p),
        None => match default_config_path() {
            Some(p) if p.exists() => parse_config_file(&p),
            _ => Ok(ScreenConfig::default()),
        },
    }
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn parse_config_file_inner(path: &Path, depth: usize) -> Result<ScreenConfig, ConfigError> {
    if depth > SOURCE_RECURSION_LIMIT {
        return Err(ConfigError {
            line: 0,
            kind: ConfigErrorKind::SourceRecursionLimit {
                path: path.display().to_string(),
                limit: SOURCE_RECURSION_LIMIT,
            },
        });
    }

    let bytes = fs::read(path).map_err(|error| ConfigError {
        line: 0,
        kind: ConfigErrorKind::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        },
    })?;
    let base_directory = path.parent().map(Path::to_owned);
    parse_config_with_sources(&bytes, &mut |source, line| {
        let source_path = source_path(source, base_directory.as_deref());
        parse_config_file_inner(&source_path, depth + 1)
            .map(Some)
            .map_err(|error| error.with_fallback_line(line))
    })
}

fn parse_config_with_sources(
    bytes: &[u8],
    source_resolver: &mut impl FnMut(&[u8], usize) -> Result<Option<ScreenConfig>, ConfigError>,
) -> Result<ScreenConfig, ConfigError> {
    let mut config = ScreenConfig::default();

    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = index + 1;
        let tokens = tokenize_line(line, line_number)?;
        let Some((command, args)) = tokens.split_first() else {
            continue;
        };

        execute_command(&mut config, command, args, line_number, source_resolver)?;
    }

    Ok(config)
}

fn one_arg<'a>(args: &'a [Vec<u8>], command: &[u8], line: usize) -> Result<&'a [u8], ConfigError> {
    if args.len() == 1 {
        Ok(&args[0])
    } else {
        Err(ConfigError {
            line,
            kind: ConfigErrorKind::WrongArgumentCount {
                command: String::from_utf8_lossy(command).into_owned(),
                expected: 1,
                actual: args.len(),
            },
        })
    }
}

fn execute_command(
    config: &mut ScreenConfig,
    command: &[u8],
    args: &[Vec<u8>],
    line: usize,
    source_resolver: &mut impl FnMut(&[u8], usize) -> Result<Option<ScreenConfig>, ConfigError>,
) -> Result<(), ConfigError> {
    match command {
        b"shell" => {
            config.shell = Some(one_arg(args, command, line)?.to_vec());
        }
        b"term" => {
            config.term = Some(one_arg(args, command, line)?.to_vec());
        }
        b"chdir" => {
            config.chdir = Some(one_arg(args, command, line)?.to_vec());
        }
        b"log" | b"deflog" => {
            config.logging = Some(log_arg(args, command, line)?);
        }
        b"logfile" => {
            config.logfile = Some(one_arg(args, command, line)?.to_vec());
        }
        b"escape" => {
            config.escape = Some(parse_escape(one_arg(args, command, line)?)?);
        }
        b"defescape" => {
            config.escape = Some(parse_escape(one_arg(args, command, line)?)?);
        }
        b"startup_message" => {
            config.startup_message = Some(bool_arg(args, command, line)?);
        }
        b"defscrollback" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.defscrollback = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"defmonitor" => {
            config.defmonitor = Some(bool_arg(args, command, line)?);
        }
        b"defflow" => {
            config.defflow = Some(bool_arg(args, command, line)?);
        }
        b"defwrap" => {
            config.defwrap = Some(bool_arg(args, command, line)?);
        }
        b"defsilence" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.defsilence = Some(text.parse::<u16>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"defautonuke" => {
            config.defautonuke = Some(bool_arg(args, command, line)?);
        }
        b"defzombie" => {
            let val = one_arg(args, command, line)?;
            config.defzombie = Some(val.to_vec());
        }
        b"hardstatus" => {
            // hardstatus takes a keyword + format; store all remaining args joined
            if args.is_empty() {
                return Err(ConfigError {
                    line,
                    kind: ConfigErrorKind::WrongArgumentCount {
                        command: "hardstatus".into(),
                        expected: 1,
                        actual: 0,
                    },
                });
            }
            config.hardstatus = Some(args.join(&b' '));
        }
        b"caption" => {
            if args.is_empty() {
                return Err(ConfigError {
                    line,
                    kind: ConfigErrorKind::WrongArgumentCount {
                        command: "caption".into(),
                        expected: 1,
                        actual: 0,
                    },
                });
            }
            config.caption = Some(args.join(&b' '));
        }
        b"select" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.select = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"termcap" | b"terminfo" => {
            // Accepted but no runtime effect for now — store as termcap override
            // Parsed as: termcap <term> <cap-string>
            if args.len() >= 2 {
                // ignore for now
            }
        }
        b"bind" => {
            if args.len() >= 2 {
                let key = args[0].clone();
                let command = args[1..].to_vec();
                config.bindings.push(KeyBinding { key, command });
            }
        }
        b"screen" | b"split" => {
            let mut window = StartupWindow::default();
            if !args.is_empty() {
                let first = &args[0];
                // Check if first arg is a number
                if let Ok(text) = std::str::from_utf8(first) {
                    if let Ok(n) = text.parse::<u32>() {
                        window.number = Some(n);
                        if args.len() > 1 {
                            window.program = Some(args[1].clone());
                            window.args = args[2..].to_vec();
                        }
                    } else {
                        window.program = Some(first.clone());
                        window.args = args[1..].to_vec();
                    }
                } else {
                    window.program = Some(first.clone());
                    window.args = args[1..].to_vec();
                }
            }
            config.startup_windows.push(window);
        }
        b"shelltitle" => {
            // Accepted — sets title pattern for new windows
        }
        b"altscreen" => {
            // Accepted
        }
        b"ignorecase" => {
            config.ignorecase = Some(bool_arg(args, command, line)?);
        }
        b"compacthist" => {
            config.compacthist = Some(bool_arg(args, command, line)?);
        }
        b"bufferfile" => {
            config.bufferfile = Some(one_arg(args, command, line)?.to_vec());
        }
        b"markkeys" => {
            config.markkeys = Some(one_arg(args, command, line)?.to_vec());
        }
        b"vbell" => {
            config.vbell = Some(bool_arg(args, command, line)?);
        }
        b"vbell_msg" => {
            config.vbell_msg = Some(one_arg(args, command, line)?.to_vec());
        }
        b"bell_msg" => {
            config.bell_msg = Some(one_arg(args, command, line)?.to_vec());
        }
        b"autodetach" => {
            config.autodetach = Some(bool_arg(args, command, line)?);
        }
        b"scrollback" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.scrollback = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"msgwait" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.msgwait = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"msgminwait" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.msgminwait = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"bce" => {
            config.bce = Some(bool_arg(args, command, line)?);
        }
        b"defutf8" => {
            config.defutf8 = Some(bool_arg(args, command, line)?);
        }
        b"defencoding" => {
            config.defencoding = Some(one_arg(args, command, line)?.to_vec());
        }
        b"slowpaste" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.slowpaste = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"sessionname" => {
            config.sessionname = Some(one_arg(args, command, line)?.to_vec());
        }
        b"password" => {
            config.password = Some(one_arg(args, command, line)?.to_vec());
        }
        b"maxwin" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.maxwin = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"defhistsize" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.defhistsize = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"crlf" => {
            config.crlf = Some(bool_arg(args, command, line)?);
        }
        b"printcmd" => {
            config.printcmd = Some(one_arg(args, command, line)?.to_vec());
        }
        b"hardcopy_append" => {
            config.hardcopy_append = Some(bool_arg(args, command, line)?);
        }
        b"nonblock" => {
            config.nonblock = Some(bool_arg(args, command, line)?);
        }
        b"zmodem" => {
            config.zmodem = Some(bool_arg(args, command, line)?);
        }
        b"wall" => {
            config.wall = Some(one_arg(args, command, line)?.to_vec());
        }
        b"backtick" => {
            if args.len() >= 3
                && let Ok(id) = std::str::from_utf8(&args[0]).unwrap_or("0").parse::<u16>()
            {
                let lifetime = match args[1].as_slice() {
                    b"0" => BacktickLifetime::Once,
                    _ => BacktickLifetime::Always,
                };
                let autorefresh = if args.len() >= 4 {
                    std::str::from_utf8(&args[2])
                        .unwrap_or("0")
                        .parse::<u32>()
                        .ok()
                } else {
                    None
                };
                let cmd_idx = if autorefresh.is_some() { 3 } else { 2 };
                let command = args[cmd_idx..].join(&b' ');
                config.backtick.push(BacktickCommand {
                    id,
                    lifetime,
                    autorefresh,
                    command,
                });
            }
        }
        b"setenv" => {
            if args.len() >= 2 {
                config.setenv.push((args[0].clone(), args[1..].join(&b' ')));
            }
        }
        b"unsetenv" => {
            if !args.is_empty() {
                config.unsetenv.push(args[0].clone());
            }
        }
        b"bindkey" => {
            // parse bindkey: bindkey [-d] [-m] [-a] [class] [args...]
            // Currently accepted with basic parsing
            let has_flag = args.iter().any(|a| a == b"-d" || a == b"-a" || a == b"-m");
            let _ = has_flag;
        }
        b"nethack" => {
            // Accepted — enables nethack mode
        }
        b"zombie" => {
            if !args.is_empty() {
                config.defzombie = Some(args.join(&b' '));
            }
        }
        b"c1" => {
            // Flow control — accepted
        }
        b"defc1" => {
            // Default flow control — accepted
        }
        b"mousetrack" => {
            // Accepted — mouse tracking mode
        }
        b"registration" => {
            // Accepted — registration message
        }
        b"defmode" => {
            // Accepted — default terminal mode
        }
        b"sorendition" => {
            // Accepted — standout rendition
        }
        b"pow_detach" => {
            // Accepted — auto-detach on power loss
        }
        b"pow_break" => {
            // Accepted — send break on power loss recovery
        }
        b"defshell" => {
            config.shell = Some(one_arg(args, command, line)?.to_vec());
        }
        b"utf8" => {
            config.defutf8 = Some(bool_arg(args, command, line)?);
        }
        b"defbce" => {
            config.bce = Some(bool_arg(args, command, line)?);
        }
        b"source" => {
            let source = one_arg(args, command, line)?;
            if let Some(source_config) = source_resolver(source, line)? {
                config.merge_from(source_config);
            }
        }
        b"multiuser" => {
            config.multiuser = Some(bool_arg(args, command, line)?);
        }
        b"acladd" => {
            if !args.is_empty() {
                let perms = if args.len() >= 2 {
                    args[1].clone()
                } else {
                    b"rwx".to_vec()
                };
                let password = if args.len() >= 3 {
                    Some(args[2].clone())
                } else {
                    None
                };
                config.acl.push(ConfigAclEntry {
                    username: args[0].clone(),
                    permissions: perms,
                    password,
                });
            }
        }
        b"aclchg" => {
            if args.len() >= 2 {
                let perms = args[1].clone();
                if let Some(entry) = config.acl.iter_mut().find(|e| e.username == args[0]) {
                    entry.permissions = perms;
                }
            }
        }
        b"acldel" if !args.is_empty() => {
            let user = &args[0];
            config.acl.retain(|e| e.username != *user);
        }
        _ => {
            // Unknown commands are silently ignored (GNU Screen behavior)
        }
    }
    Ok(())
}

/// Parse an escape sequence string like "^Aa" or "^Aa" into raw bytes.
/// "^A" becomes 0x01 (Ctrl-A), literal characters follow.
fn parse_escape(input: &[u8]) -> Result<Vec<u8>, ConfigError> {
    let mut result = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'^' && i + 1 < input.len() {
            let next = input[i + 1];
            if next.is_ascii_uppercase() {
                result.push(next - b'A' + 1);
            } else if next == b'?' {
                result.push(0x7f);
            } else {
                result.push(next & 0x1f);
            }
            i += 2;
        } else {
            result.push(input[i]);
            i += 1;
        }
    }
    Ok(result)
}

fn log_arg(args: &[Vec<u8>], command: &[u8], line: usize) -> Result<bool, ConfigError> {
    match args {
        [] => Ok(true),
        [value] if value == b"on" => Ok(true),
        [value] if value == b"off" => Ok(false),
        [value] => Err(ConfigError {
            line,
            kind: ConfigErrorKind::InvalidArgument {
                command: String::from_utf8_lossy(command).into_owned(),
                value: String::from_utf8_lossy(value).into_owned(),
            },
        }),
        _ => Err(ConfigError {
            line,
            kind: ConfigErrorKind::WrongArgumentCount {
                command: String::from_utf8_lossy(command).into_owned(),
                expected: 1,
                actual: args.len(),
            },
        }),
    }
}

fn bool_arg(args: &[Vec<u8>], command: &[u8], line: usize) -> Result<bool, ConfigError> {
    match args {
        [] => Ok(true),
        [value] if value == b"on" => Ok(true),
        [value] if value == b"off" => Ok(false),
        [value] => Err(ConfigError {
            line,
            kind: ConfigErrorKind::InvalidArgument {
                command: String::from_utf8_lossy(command).into_owned(),
                value: String::from_utf8_lossy(value).into_owned(),
            },
        }),
        _ => Err(ConfigError {
            line,
            kind: ConfigErrorKind::WrongArgumentCount {
                command: String::from_utf8_lossy(command).into_owned(),
                expected: 1,
                actual: args.len(),
            },
        }),
    }
}

// ---------------------------------------------------------------------------
// Merging
// ---------------------------------------------------------------------------

impl ScreenConfig {
    fn merge_from(&mut self, other: Self) {
        if other.shell.is_some() {
            self.shell = other.shell;
        }
        if other.term.is_some() {
            self.term = other.term;
        }
        if other.chdir.is_some() {
            self.chdir = other.chdir;
        }
        if other.logging.is_some() {
            self.logging = other.logging;
        }
        if other.logfile.is_some() {
            self.logfile = other.logfile;
        }
        if other.escape.is_some() {
            self.escape = other.escape;
        }
        if other.startup_message.is_some() {
            self.startup_message = other.startup_message;
        }
        if other.defscrollback.is_some() {
            self.defscrollback = other.defscrollback;
        }
        if other.defmonitor.is_some() {
            self.defmonitor = other.defmonitor;
        }
        if other.defflow.is_some() {
            self.defflow = other.defflow;
        }
        if other.defwrap.is_some() {
            self.defwrap = other.defwrap;
        }
        if other.defsilence.is_some() {
            self.defsilence = other.defsilence;
        }
        if other.defautonuke.is_some() {
            self.defautonuke = other.defautonuke;
        }
        if other.defzombie.is_some() {
            self.defzombie = other.defzombie;
        }
        if other.hardstatus.is_some() {
            self.hardstatus = other.hardstatus;
        }
        if other.caption.is_some() {
            self.caption = other.caption;
        }
        if other.select.is_some() {
            self.select = other.select;
        }
        if other.ignorecase.is_some() {
            self.ignorecase = other.ignorecase;
        }
        if other.compacthist.is_some() {
            self.compacthist = other.compacthist;
        }
        if other.bufferfile.is_some() {
            self.bufferfile = other.bufferfile;
        }
        if other.markkeys.is_some() {
            self.markkeys = other.markkeys;
        }
        if other.vbell.is_some() {
            self.vbell = other.vbell;
        }
        if other.vbell_msg.is_some() {
            self.vbell_msg = other.vbell_msg;
        }
        if other.bell_msg.is_some() {
            self.bell_msg = other.bell_msg;
        }
        if other.autodetach.is_some() {
            self.autodetach = other.autodetach;
        }
        if other.scrollback.is_some() {
            self.scrollback = other.scrollback;
        }
        if other.msgwait.is_some() {
            self.msgwait = other.msgwait;
        }
        if other.msgminwait.is_some() {
            self.msgminwait = other.msgminwait;
        }
        if other.bce.is_some() {
            self.bce = other.bce;
        }
        if other.defutf8.is_some() {
            self.defutf8 = other.defutf8;
        }
        if other.defencoding.is_some() {
            self.defencoding = other.defencoding;
        }
        if other.slowpaste.is_some() {
            self.slowpaste = other.slowpaste;
        }
        if other.sessionname.is_some() {
            self.sessionname = other.sessionname;
        }
        if other.password.is_some() {
            self.password = other.password;
        }
        if other.maxwin.is_some() {
            self.maxwin = other.maxwin;
        }
        if other.defhistsize.is_some() {
            self.defhistsize = other.defhistsize;
        }
        if other.crlf.is_some() {
            self.crlf = other.crlf;
        }
        if other.printcmd.is_some() {
            self.printcmd = other.printcmd;
        }
        if other.hardcopy_append.is_some() {
            self.hardcopy_append = other.hardcopy_append;
        }
        if other.nonblock.is_some() {
            self.nonblock = other.nonblock;
        }
        if other.zmodem.is_some() {
            self.zmodem = other.zmodem;
        }
        if other.wall.is_some() {
            self.wall = other.wall;
        }
        self.backtick.extend(other.backtick);
        self.setenv.extend(other.setenv);
        self.unsetenv.extend(other.unsetenv);
        if other.multiuser.is_some() {
            self.multiuser = other.multiuser;
        }
        self.acl.extend(other.acl);
        self.bindings.extend(other.bindings);
        self.startup_windows.extend(other.startup_windows);
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

fn tokenize_line(line: &[u8], line_number: usize) -> Result<Vec<Vec<u8>>, ConfigError> {
    let mut tokens = Vec::new();
    let mut current = Vec::new();
    let mut quote = None;
    let mut escaped = false;

    for byte in trim_trailing_cr(line) {
        if escaped {
            current.push(*byte);
            escaped = false;
            continue;
        }

        if *byte == b'\\' {
            escaped = true;
            continue;
        }

        if let Some(quote_byte) = quote {
            if *byte == quote_byte {
                quote = None;
            } else {
                current.push(*byte);
            }
            continue;
        }

        match *byte {
            b'\'' | b'"' => quote = Some(*byte),
            b'#' => break,
            b' ' | b'\t' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(*byte),
        }
    }

    if escaped {
        current.push(b'\\');
    }
    if let Some(quote_byte) = quote {
        return Err(ConfigError {
            line: line_number,
            kind: ConfigErrorKind::UnterminatedQuote { quote: quote_byte },
        });
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

fn trim_trailing_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

// ---------------------------------------------------------------------------
// Default config discovery
// ---------------------------------------------------------------------------

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Source resolution
// ---------------------------------------------------------------------------

fn source_path(source: &[u8], base_directory: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(OsString::from_vec(source.to_vec()));
    if path.is_absolute() {
        path
    } else if let Some(base_directory) = base_directory {
        base_directory.join(path)
    } else {
        path
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub line: usize,
    pub kind: ConfigErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigErrorKind {
    UnterminatedQuote {
        quote: u8,
    },
    InvalidArgument {
        command: String,
        value: String,
    },
    Io {
        path: String,
        message: String,
    },
    SourceRecursionLimit {
        path: String,
        limit: usize,
    },
    WrongArgumentCount {
        command: String,
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ConfigErrorKind::UnterminatedQuote { quote } => write!(
                formatter,
                "line {}: unterminated {} quote",
                self.line, *quote as char
            ),
            ConfigErrorKind::InvalidArgument { command, value } => write!(
                formatter,
                "line {}: unsupported {command} argument {value}",
                self.line
            ),
            ConfigErrorKind::Io { path, message } => write!(formatter, "{path}: {message}"),
            ConfigErrorKind::SourceRecursionLimit { path, limit } => write!(
                formatter,
                "{path}: source recursion exceeded configured limit of {limit}"
            ),
            ConfigErrorKind::WrongArgumentCount {
                command,
                expected,
                actual,
            } => write!(
                formatter,
                "line {}: {command} expects {expected} argument(s), got {actual}",
                self.line
            ),
        }
    }
}

impl Error for ConfigError {}

impl ConfigError {
    fn with_fallback_line(mut self, line: usize) -> Self {
        if self.line == 0 {
            self.line = line;
        }
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_shell_and_term_commands() {
        let config = parse_config(
            b"shell /tmp/sh\nterm screen-256color\nchdir /tmp\nlog on\nlogfile /tmp/screen.log\n",
        )
        .expect("config parses");

        assert_eq!(config.shell, Some(b"/tmp/sh".to_vec()));
        assert_eq!(config.term, Some(b"screen-256color".to_vec()));
        assert_eq!(config.chdir, Some(b"/tmp".to_vec()));
        assert_eq!(config.logging, Some(true));
        assert_eq!(config.logfile, Some(b"/tmp/screen.log".to_vec()));
    }

    #[test]
    fn tokenization_supports_quotes_escapes_comments_and_crlf() {
        let config = parse_config(
            b"# ignored\r\nshell \"/tmp/custom shell\" # comment\r\nterm screen\\-direct\r\nchdir '/tmp/a dir'\r\ndeflog off\r\nlogfile '/tmp/a log'\r\n",
        )
        .expect("config parses");

        assert_eq!(config.shell, Some(b"/tmp/custom shell".to_vec()));
        assert_eq!(config.term, Some(b"screen-direct".to_vec()));
        assert_eq!(config.chdir, Some(b"/tmp/a dir".to_vec()));
        assert_eq!(config.logging, Some(false));
        assert_eq!(config.logfile, Some(b"/tmp/a log".to_vec()));
    }

    #[test]
    fn bare_log_enables_logging() {
        let config = parse_config(b"log\n").expect("config parses");
        assert_eq!(config.logging, Some(true));
    }

    #[test]
    fn parses_escape_sequence() {
        let config = parse_config(b"escape ^Aa\n").expect("config parses");
        assert_eq!(config.escape, Some(vec![0x01, b'a']));
    }

    #[test]
    fn parses_escape_sequence_different_prefix() {
        let config = parse_config(b"escape ^Bb\n").expect("config parses");
        assert_eq!(config.escape, Some(vec![0x02, b'b']));
    }

    #[test]
    fn parses_startup_message_off() {
        let config = parse_config(b"startup_message off\n").expect("config parses");
        assert_eq!(config.startup_message, Some(false));
    }

    #[test]
    fn parses_defscrollback() {
        let config = parse_config(b"defscrollback 5000\n").expect("config parses");
        assert_eq!(config.defscrollback, Some(5000));
    }

    #[test]
    fn parses_hardstatus_and_caption() {
        let config = parse_config(b"hardstatus alwayslastline \"%H\"\ncaption always \"%?%F%{=b bc}%:%{= bw}%?%n %t%='\"\n")
            .expect("config parses");
        // Args joined with space: "alwayslastline %H"
        assert_eq!(config.hardstatus, Some(b"alwayslastline %H".to_vec()));
        assert!(!config.caption.as_ref().unwrap().is_empty());
    }

    #[test]
    fn parses_bindings() {
        let config =
            parse_config(b"bind c screen\nbind k kill\nbind ^K kill\n").expect("config parses");
        assert_eq!(config.bindings.len(), 3);
        assert_eq!(config.bindings[0].key, b"c");
        assert_eq!(config.bindings[0].command, vec![b"screen".to_vec()]);
        assert_eq!(config.bindings[1].key, b"k");
        assert_eq!(config.bindings[2].key, b"^K");
        assert_eq!(config.bindings[2].command, vec![b"kill".to_vec()]);
    }

    #[test]
    fn parses_startup_screen_windows() {
        let config =
            parse_config(b"screen 1 top\nscreen 2 less /etc/passwd\n").expect("config parses");
        assert_eq!(config.startup_windows.len(), 2);
        assert_eq!(config.startup_windows[0].number, Some(1));
        assert_eq!(config.startup_windows[0].program, Some(b"top".to_vec()));
        assert_eq!(config.startup_windows[1].number, Some(2));
        assert_eq!(config.startup_windows[1].program, Some(b"less".to_vec()));
        assert_eq!(
            config.startup_windows[1].args,
            vec![b"/etc/passwd".to_vec()]
        );
    }

    #[test]
    fn parses_select() {
        let config = parse_config(b"select 1\n").expect("config parses");
        assert_eq!(config.select, Some(1));
    }

    #[test]
    fn termcap_and_terminfo_accepted_but_ignored() {
        let config = parse_config(
            b"termcap xterm 'AF=\\E[3%dm:AB=\\E[4%dm'\nterminfo xterm 'AF=\\E[3%dm'\n",
        )
        .expect("config parses");
        // Should not panic, just be absorbed
        assert!(config.term.is_none());
    }

    #[test]
    fn source_file_merges_startup_config() {
        let directory = temp_directory("source");
        std::fs::create_dir(&directory).expect("create temp config directory");
        let included = directory.join("included");
        let root = directory.join("screenrc");
        std::fs::write(
            &included,
            b"shell /tmp/from-source\nterm screen-256color\nlogfile /tmp/source.log\n",
        )
        .expect("write included");
        std::fs::write(
            &root,
            format!("chdir /tmp/before\nsource {}\nlog on\n", included.display()),
        )
        .expect("write root");

        let config = parse_config_file(&root).expect("source config parses");
        let _ = std::fs::remove_dir_all(directory);

        assert_eq!(config.shell, Some(b"/tmp/from-source".to_vec()));
        assert_eq!(config.term, Some(b"screen-256color".to_vec()));
        assert_eq!(config.chdir, Some(b"/tmp/before".to_vec()));
        assert_eq!(config.logging, Some(true));
        assert_eq!(config.logfile, Some(b"/tmp/source.log".to_vec()));
    }

    #[test]
    fn preserves_non_utf8_argument_bytes() {
        let config = parse_config(b"shell /tmp/\xff-shell\n").expect("config parses");
        assert_eq!(config.shell, Some(b"/tmp/\xff-shell".to_vec()));
    }

    #[test]
    fn rejects_unterminated_quotes() {
        let error = parse_config(b"shell \"/tmp/sh\n").expect_err("quote error");
        assert_eq!(
            error,
            ConfigError {
                line: 1,
                kind: ConfigErrorKind::UnterminatedQuote { quote: b'"' },
            }
        );
    }

    #[test]
    fn rejects_wrong_argument_count_for_supported_commands() {
        let error = parse_config(b"term one two\n").expect_err("argument error");
        assert_eq!(
            error,
            ConfigError {
                line: 1,
                kind: ConfigErrorKind::WrongArgumentCount {
                    command: "term".to_owned(),
                    expected: 1,
                    actual: 2,
                },
            }
        );
    }

    #[test]
    fn rejects_invalid_log_argument() {
        let error = parse_config(b"log maybe\n").expect_err("log value error");
        assert_eq!(
            error,
            ConfigError {
                line: 1,
                kind: ConfigErrorKind::InvalidArgument {
                    command: "log".to_owned(),
                    value: "maybe".to_owned(),
                },
            }
        );
    }

    #[test]
    fn ignores_unknown_commands() {
        let config = parse_config(b"unknown_command arg1 arg2\nshell /bin/sh\n")
            .expect("unknown commands ignored");
        assert_eq!(config.shell, Some(b"/bin/sh".to_vec()));
    }

    #[test]
    fn empty_config_is_default() {
        let config = parse_config(b"").expect("empty config parses");
        assert_eq!(config, ScreenConfig::default());
    }

    fn temp_directory(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("sc-cfg-{name}-{}-{nanos}", std::process::id()))
    }
}
