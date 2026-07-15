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
    /// Termcap/terminfo overrides: (term_name, capability_string).
    pub termcap_overrides: Vec<(Vec<u8>, Vec<u8>)>,
    /// Zmodem catch support.
    pub zmodem: Option<bool>,
    /// Mouse tracking mode.
    pub mousetrack: Option<bool>,
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
    /// Idle timeout in seconds before blanking (0 = disabled).
    pub idle: Option<u32>,
    /// Blanker program (run when idling, instead of blank).
    pub blanker: Option<Vec<u8>>,
    /// Blanker program arguments.
    pub blankerprg: Option<Vec<u8>>,
    /// Nethack mode (disable auto-wrap).
    pub nethack: Option<bool>,
    /// Standout rendition mode.
    pub sorendition: Option<bool>,
    /// Window group name.
    pub group: Option<Vec<u8>>,
    /// Layout directory for save/restore.
    pub layoutdir: Option<Vec<u8>>,
    /// Activity notification message format.
    pub activity: Option<Vec<u8>>,
    /// Bell action ("all", "visible", "audible", or "none").
    pub bell_action: Option<Vec<u8>>,
    /// Hardcopy directory for hardcopy/screen dump.
    pub hardcopydir: Option<Vec<u8>>,
    /// Log timestamp format (after seconds).
    pub logtstamp: Option<Vec<u8>>,
    /// Default login state for new windows.
    pub deflogin: Option<bool>,
    /// Attribute color mapping: attrcolor <attr> [<color-pair>].
    pub attrcolor: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    /// All partial refresh mode.
    pub allpartial: Option<bool>,
    /// Auto-focus new windows.
    pub autofocus: Option<bool>,
    /// Default charset for new windows.
    pub defcharset: Option<Vec<u8>>,
    /// Silence notification config.
    pub silence: Option<Vec<u8>>,
    /// Silence notification wait time in tenths of a second.
    pub silencewait: Option<u32>,
    /// Soft wrap mode for new windows.
    pub wrap: Option<bool>,
    /// Wrap at specific column for new windows.
    pub wrapsize: Option<u32>,
    /// Break duration config: breaktype <duration-in-ms>.
    pub breaktype: Option<u32>,
    /// Default mouse tracking for new windows.
    pub defmousetrack: Option<bool>,
    /// Suppress window list during key binding (GNU Screen show/hide).
    pub windowlist: Option<bool>,
    /// Output encoding for a display.
    pub encoding: Option<Vec<u8>>,
    // ── Additional config fields from audit ──
    /// ACL group: add user to named group.
    pub aclgrp: Option<Vec<Vec<u8>>>,
    /// Switch user for multiuser mode.
    pub su: Option<Vec<u8>>,
    /// Umask for socket creation.
    pub umask: Option<u32>,
    /// Default non-blocking I/O for new windows.
    pub defnonblock: Option<bool>,
    /// Lock screen command.
    pub lockscreen: Option<bool>,
    /// Partial redraw mode.
    pub partial: Option<bool>,
    /// Write lock for multiuser mode.
    pub writelock: Option<Vec<u8>>,
    /// Show time on status line.
    pub time: Option<bool>,
    /// Default hardstatus for new windows (defhstatus format string).
    pub defhstatus: Option<Vec<u8>>,
    /// Default output buffer limit in bytes (defobuflimit).
    pub defobuflimit: Option<usize>,
    /// CJK ambiguous-width handling (cjkwidth on/off).
    pub cjkwidth: Option<bool>,
    /// Caption/screen rendition for flagged windows: (flag, attr, color).
    pub rendition: Vec<RenditionRule>,
    /// Keys to unbind via `unbind` command.
    pub unbind_keys: Vec<Vec<u8>>,
    /// Keys to unbind via `unbindkey` command.
    pub unbindkey_keys: Vec<Vec<u8>>,
    /// Window width (columns) from `width` command.
    pub width: Option<u32>,
    /// Debug mode from `debug` command.
    pub debug: Option<bool>,
    /// Login mode for windows (separate from deflogin default).
    pub login: Option<Vec<u8>>,
    /// Buffer size in bytes from `bufsize` command.
    pub bufsize: Option<u32>,
    /// Layout save/restore commands.
    pub layout: Vec<Vec<u8>>,
    /// Window title from `title` command (runtime).
    pub title: Option<Vec<u8>>,
    /// Monitor command toggle message.
    pub monitor: Option<Vec<u8>>,
    /// Stuff text into window at startup.
    pub stuff: Option<Vec<u8>>,
    /// Eval multiline command string at startup.
    pub eval_cmds: Vec<Vec<u8>>,
    /// Execute program at startup (exec command).
    pub exec_cmds: Vec<Vec<u8>>,
    /// At-commands: (window_number, command_string).
    pub at_cmds: Vec<(Vec<u8>, Vec<u8>)>,
    /// Copy mode entry from .screenrc.
    pub copy: Option<Vec<u8>>,
    /// Paste buffer content from .screenrc.
    pub paste: Option<Vec<u8>>,
    /// Register operations: (register_byte, content).
    pub register: Vec<(Vec<u8>, Vec<u8>)>,
    /// Read register from file: (register_byte, file_path).
    pub readreg: Vec<(Vec<u8>, Vec<u8>)>,
    /// Write register to file: (register_byte, file_path).
    pub writereg: Vec<(Vec<u8>, Vec<u8>)>,
    /// Write buffer to file: file_path.
    pub writebuf: Option<Vec<u8>>,
    /// Read buffer from file: file_path.
    pub readbuf: Option<Vec<u8>>,
    /// Remove buffer file: file_path.
    pub removebuf: Option<Vec<u8>>,
    /// Default keymap name.
    pub defkmap: Option<Vec<u8>>,
    /// Default command (for keybinding fallback).
    pub defcmnd: Option<Vec<u8>>,
    /// Default list format.
    pub deflist: Option<Vec<u8>>,
    /// Window type default.
    pub deftype: Option<Vec<u8>>,
    /// Default auto parameter.
    pub defautoparam: Option<Vec<u8>>,
    /// Default pan position.
    pub defpanposition: Option<Vec<u8>>,
    /// Focus region number from `focus` command.
    pub focus: Option<Vec<u8>>,
    /// Clear screen command (just toggle).
    pub clear_screen: Option<bool>,
    /// Dump terminal state command.
    pub dump: Option<Vec<u8>>,
    /// Schedule command: (time, command_string).
    pub sched: Vec<(Vec<u8>, Vec<u8>)>,
    /// Deselect window (shorthand).
    pub deselect: Option<Vec<u8>>,
    /// Current window info command.
    pub currwin: Option<Vec<u8>>,
    /// Default buffer limit.
    pub defbufflim: Option<usize>,
    /// Hstatus alias for hardstatus.
    pub hstatus: Option<Vec<u8>>,
    /// ANSI partial mode.
    pub ansi_partial: Option<bool>,
    /// Auto refresh for backtick (separate field).
    pub autorefresh: Option<u32>,
    /// Default charset alias.
    pub charset: Option<Vec<u8>>,
    /// Flow control from `flow` command.
    pub flow_cmd: Option<bool>,
    /// XON/XOFF character definitions.
    pub xon: Option<Vec<u8>>,
    pub xoff: Option<Vec<u8>>,
    /// Command char (colon) configuration.
    pub colon: Option<Vec<u8>>,
    /// Keymap configuration.
    pub kmap: Option<Vec<u8>>,
    /// Key buffer size.
    pub keybuf: Option<u32>,
    /// Output buffer allocation.
    pub obufalloc: Option<u32>,
    /// Output buffer count.
    pub obufcount: Option<u32>,
    /// Output buffer wait time.
    pub obufwait: Option<u32>,
    /// Dense display mode.
    pub dense: Option<bool>,
    /// Map default command.
    pub mapdefault: Option<Vec<u8>>,
    /// Map next command.
    pub mapnext: Option<Vec<u8>>,
    /// Predicate condition list.
    pub pred: Vec<Vec<u8>>,
}

/// A rendition rule for the caption/screen display.
/// Format: `rendition <flag> <attr> [color]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenditionRule {
    /// Flag name ("bell", "monitor", "silence", "so").
    pub flag: Vec<u8>,
    /// Attribute string (e.g. "rv", "ul", "bl", "+b").
    pub attr: Option<Vec<u8>>,
    /// Optional color specification.
    pub color: Option<Vec<u8>>,
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
            // Store termcap/terminfo override: termcap <term> <cap-string>
            if args.len() >= 2 {
                config
                    .termcap_overrides
                    .push((args[0].clone(), args[1].clone()));
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
            // bindkey [-d] [-m] [-a] [class] [string [command args...]]
            // For now, treat the first non-flag argument as the key and rest as command
            let non_flags: Vec<_> = args
                .iter()
                .filter(|a| {
                    **a != b"-d" && **a != b"-a" && **a != b"-m" && **a != b"-k" && **a != b"-t"
                })
                .collect();
            if non_flags.len() >= 2 {
                #[allow(clippy::collapsible_if)]
                if let Ok(key) = parse_escape(non_flags[0]) {
                    let command = non_flags[1..].iter().map(|a| a.to_vec()).collect();
                    config.bindings.push(KeyBinding { key, command });
                }
            }
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
            config.mousetrack = Some(true);
        }
        b"nomousetrack" => {
            config.mousetrack = Some(false);
        }
        b"registration" => {
            // Accepted — registration message
        }
        b"defmode" => {
            // Accepted — default terminal mode
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
        b"idle" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.idle = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"blanker" => {
            config.blanker = Some(one_arg(args, command, line)?.to_vec());
        }
        b"blankerprg" => {
            config.blankerprg = Some(one_arg(args, command, line)?.to_vec());
        }
        b"nethack" => {
            config.nethack = Some(bool_arg(args, command, line)?);
        }
        b"sorendition" => {
            config.sorendition = Some(bool_arg(args, command, line)?);
        }
        b"group" => {
            config.group = Some(one_arg(args, command, line)?.to_vec());
        }
        b"layoutdir" => {
            config.layoutdir = Some(one_arg(args, command, line)?.to_vec());
        }
        b"activity" => {
            config.activity = Some(one_arg(args, command, line)?.to_vec());
        }
        b"hardcopydir" => {
            config.hardcopydir = Some(one_arg(args, command, line)?.to_vec());
        }
        b"logtstamp" => {
            if !args.is_empty() {
                config.logtstamp = Some(args.join(&b' '));
            }
        }
        b"deflogin" => {
            config.deflogin = Some(bool_arg(args, command, line)?);
        }
        b"attrcolor" => {
            if args.len() >= 2 {
                let attr = args[0].clone();
                let color = if args.len() >= 3 {
                    Some(args[1..].join(&b' '))
                } else {
                    Some(args[1].clone())
                };
                config.attrcolor.push((attr, color));
            } else if !args.is_empty() {
                config.attrcolor.push((args[0].clone(), None));
            }
        }
        b"allpartial" => {
            config.allpartial = Some(bool_arg(args, command, line)?);
        }
        b"autofocus" => {
            config.autofocus = Some(bool_arg(args, command, line)?);
        }
        b"defcharset" => {
            config.defcharset = Some(one_arg(args, command, line)?.to_vec());
        }
        b"silence" => {
            if !args.is_empty() {
                config.silence = Some(args.join(&b' '));
            }
        }
        b"silencewait" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.silencewait = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"wrap" => {
            config.wrap = Some(bool_arg(args, command, line)?);
        }
        b"wrapsize" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.wrapsize = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"breaktype" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.breaktype = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"defmousetrack" => {
            config.defmousetrack = Some(bool_arg(args, command, line)?);
        }
        b"bell" => {
            if !args.is_empty() {
                config.bell_action = Some(args[0].clone());
            }
        }
        b"echo" => {
            // Accepted — startup/runtime display command.
        }
        b"sleep" => {
            // Accepted — startup delay command.
        }
        b"number" => {
            // Accepted — window number change command.
        }
        b"sort" => {
            // Accepted — window sorting.
        }
        b"process" => {
            // Accepted — process group management.
        }
        b"version" => {
            // Accepted — version display.
        }
        b"next" => {
            // Accepted — next window switch.
        }
        b"prev" => {
            // Accepted — previous window switch.
        }
        b"other" => {
            // Accepted — other window switch.
        }
        b"detach" => {
            // Accepted — detach session.
        }
        b"kill" => {
            // Accepted — kill window.
        }
        b"quit" => {
            // Accepted — quit session.
        }
        b"hardcopy" => {
            // Accepted — hardcopy/screendump.
        }
        b"digraph" => {
            // Accepted — digraph display.
        }
        b"fit" => {
            // Accepted — fit window to region.
        }
        b"only" => {
            // Accepted — only window mode.
        }
        b"remove" => {
            // Accepted — remove display.
        }
        b"reset" => {
            // Accepted — reset terminal.
        }
        b"key" => {
            // Accepted — send key to bindings.
        }
        b"verbose" => {
            // Accepted — verbose file creation.
        }
        b"dinfo" => {
            // Accepted — default info.
        }
        b"windowlist" => {
            config.windowlist = Some(bool_arg(args, command, line)?);
        }
        b"encoding" => {
            config.encoding = Some(one_arg(args, command, line)?.to_vec());
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
        b"defhstatus" => {
            config.defhstatus = Some(one_arg(args, command, line)?.to_vec());
        }
        b"defobuflimit" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.defobuflimit = Some(text.parse::<usize>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"defnonblock" => {
            config.nonblock = Some(bool_arg(args, command, line)?);
        }
        b"cjkwidth" => {
            config.cjkwidth = Some(bool_arg(args, command, line)?);
        }
        b"rendition" => {
            if !args.is_empty() {
                let flag = args[0].clone();
                let attr = args.get(1).cloned();
                let color = if args.len() >= 3 {
                    Some(args[2..].join(&b' '))
                } else {
                    None
                };
                config.rendition.push(RenditionRule { flag, attr, color });
            }
        }
        // ── Unbind commands ──
        b"unbind" => {
            if !args.is_empty() {
                config.unbind_keys.push(args[0].clone());
            }
        }
        b"unbindkey" => {
            if !args.is_empty() {
                config.unbindkey_keys.push(args[0].clone());
            }
        }
        // ── Config-storing commands (new fields) ──
        b"hstatus" => {
            if !args.is_empty() {
                config.hstatus = Some(args.join(&b' '));
            }
        }
        b"defbufflim" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.defbufflim = Some(text.parse::<usize>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"width" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.width = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"debug" => {
            config.debug = Some(bool_arg(args, command, line)?);
        }
        b"login" => {
            if !args.is_empty() {
                config.login = Some(args.join(&b' '));
            }
        }
        b"bufsize" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.bufsize = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"clear" => {
            config.clear_screen = Some(true);
        }
        b"set" if args.len() >= 2 => {
            // set <variable> <value> - store as generic variable
            config.setenv.push((args[0].clone(), args[1..].join(&b' ')));
        }
        b"layout" => {
            config.layout.push(args.join(&b' '));
        }
        b"sched" if args.len() >= 2 => {
            config.sched.push((args[0].clone(), args[1..].join(&b' ')));
        }
        b"title" => {
            if !args.is_empty() {
                config.title = Some(args.join(&b' '));
            }
        }
        b"monitor" => {
            if !args.is_empty() {
                config.monitor = Some(args.join(&b' '));
            }
        }
        b"stuff" => {
            config.stuff = Some(args.join(&b' '));
        }
        b"eval" => {
            config.eval_cmds.push(args.join(&b' '));
        }
        b"exec" => {
            config.exec_cmds.push(args.join(&b' '));
        }
        b"at" if args.len() >= 2 => {
            config
                .at_cmds
                .push((args[0].clone(), args[1..].join(&b' ')));
        }
        b"copy" => {
            if !args.is_empty() {
                config.copy = Some(args.join(&b' '));
            }
        }
        b"paste" => {
            if !args.is_empty() {
                config.paste = Some(args.join(&b' '));
            }
        }
        b"register" if args.len() >= 2 => {
            config
                .register
                .push((args[0].clone(), args[1..].join(&b' ')));
        }
        b"readreg" if args.len() >= 2 => {
            config
                .readreg
                .push((args[0].clone(), args[1..].join(&b' ')));
        }
        b"writereg" if args.len() >= 2 => {
            config
                .writereg
                .push((args[0].clone(), args[1..].join(&b' ')));
        }
        b"writebuf" => {
            if !args.is_empty() {
                config.writebuf = Some(args.join(&b' '));
            }
        }
        b"readbuf" => {
            if !args.is_empty() {
                config.readbuf = Some(args.join(&b' '));
            }
        }
        b"removebuf" => {
            if !args.is_empty() {
                config.removebuf = Some(args.join(&b' '));
            }
        }
        b"defkmap" => {
            config.defkmap = Some(one_arg(args, command, line)?.to_vec());
        }
        b"defcmnd" => {
            config.defcmnd = Some(one_arg(args, command, line)?.to_vec());
        }
        b"deflist" => {
            config.deflist = Some(one_arg(args, command, line)?.to_vec());
        }
        b"deftype" => {
            config.deftype = Some(one_arg(args, command, line)?.to_vec());
        }
        b"defautoparam" => {
            config.defautoparam = Some(one_arg(args, command, line)?.to_vec());
        }
        b"defpanposition" => {
            config.defpanposition = Some(one_arg(args, command, line)?.to_vec());
        }
        b"focus" => {
            if !args.is_empty() {
                config.focus = Some(args.join(&b' '));
            }
        }
        b"dump" => {
            if !args.is_empty() {
                config.dump = Some(args.join(&b' '));
            }
        }
        b"deselect" => {
            if !args.is_empty() {
                config.deselect = Some(args.join(&b' '));
            }
        }
        b"currwin" => {
            if !args.is_empty() {
                config.currwin = Some(args.join(&b' '));
            }
        }
        b"ansi_partial" => {
            config.ansi_partial = Some(bool_arg(args, command, line)?);
        }
        b"autorefresh" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.autorefresh = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"charset" => {
            config.charset = Some(one_arg(args, command, line)?.to_vec());
        }
        b"flow" => {
            config.flow_cmd = Some(bool_arg(args, command, line)?);
        }
        b"xon" => {
            if !args.is_empty() {
                config.xon = Some(one_arg(args, command, line)?.to_vec());
            }
        }
        b"xoff" => {
            if !args.is_empty() {
                config.xoff = Some(one_arg(args, command, line)?.to_vec());
            }
        }
        b"colon" => {
            if !args.is_empty() {
                config.colon = Some(one_arg(args, command, line)?.to_vec());
            }
        }
        b"kmap" => {
            config.kmap = Some(one_arg(args, command, line)?.to_vec());
        }
        b"keybuf" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.keybuf = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"obufalloc" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.obufalloc = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"obufcount" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.obufcount = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"obufwait" => {
            let val = one_arg(args, command, line)?;
            let text = std::str::from_utf8(val).map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: String::from_utf8_lossy(val).into_owned(),
                },
            })?;
            config.obufwait = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"dense" => {
            config.dense = Some(bool_arg(args, command, line)?);
        }
        b"mapdefault" => {
            if !args.is_empty() {
                config.mapdefault = Some(args.join(&b' '));
            }
        }
        b"mapnext" => {
            if !args.is_empty() {
                config.mapnext = Some(args.join(&b' '));
            }
        }
        b"pred" => {
            config.pred.push(args.join(&b' '));
        }
        // ── Explicit no-ops for runtime-only commands (recognized, silently accepted) ──
        b"help" => {}
        b"info" => {}
        b"lastmsg" => {}
        b"license" => {}
        b"redisplay" => {}
        b"search" => {}
        b"suspend" => {}
        b"wipe" => {}
        b"windows" | b"winlist" => {}
        b"break" => {}
        // ── Additional commands from audit ──
        b"aclgrp" => {
            config.aclgrp = Some(args.iter().cloned().collect());
        }
        b"su" => {
            config.su = Some(one_arg(args, command, line)?.to_vec());
        }
        b"umask" => {
            let text = std::str::from_utf8(one_arg(args, command, line)?).map_err(|_| {
                ConfigError {
                    line,
                    kind: ConfigErrorKind::InvalidArgument {
                        command: String::from_utf8_lossy(command).into_owned(),
                        value: "<non-utf8>".to_owned(),
                    },
                }
            })?;
            config.umask = Some(text.parse::<u32>().map_err(|_| ConfigError {
                line,
                kind: ConfigErrorKind::InvalidArgument {
                    command: String::from_utf8_lossy(command).into_owned(),
                    value: text.to_owned(),
                },
            })?);
        }
        b"lockscreen" | b"lock" => {
            config.lockscreen = Some(true);
        }
        b"partial" => {
            config.partial = Some(bool_arg(args, command, line)?);
        }
        b"writelock" => {
            config.writelock = Some(args.join(&b' '));
        }
        b"time" => {
            config.time = Some(true);
        }
        b"ins_reg" => {
            // Insert register: accepted, runtime only.
        }
        // ── Obsolete/rare commands accepted as no-op ──
        b"autoparam" => {}
        b"font" => {}
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
        if other.layoutdir.is_some() {
            self.layoutdir = other.layoutdir;
        }
        if other.activity.is_some() {
            self.activity = other.activity;
        }
        if other.bell_action.is_some() {
            self.bell_action = other.bell_action;
        }
        if other.hardcopydir.is_some() {
            self.hardcopydir = other.hardcopydir;
        }
        if other.logtstamp.is_some() {
            self.logtstamp = other.logtstamp;
        }
        if other.deflogin.is_some() {
            self.deflogin = other.deflogin;
        }
        if other.allpartial.is_some() {
            self.allpartial = other.allpartial;
        }
        if other.autofocus.is_some() {
            self.autofocus = other.autofocus;
        }
        if other.defcharset.is_some() {
            self.defcharset = other.defcharset;
        }
        if other.silence.is_some() {
            self.silence = other.silence;
        }
        if other.silencewait.is_some() {
            self.silencewait = other.silencewait;
        }
        if other.wrap.is_some() {
            self.wrap = other.wrap;
        }
        if other.wrapsize.is_some() {
            self.wrapsize = other.wrapsize;
        }
        if other.breaktype.is_some() {
            self.breaktype = other.breaktype;
        }
        if other.defmousetrack.is_some() {
            self.defmousetrack = other.defmousetrack;
        }
        if other.windowlist.is_some() {
            self.windowlist = other.windowlist;
        }
        if other.encoding.is_some() {
            self.encoding = other.encoding;
        }
        if other.aclgrp.is_some() {
            self.aclgrp = other.aclgrp;
        }
        if other.su.is_some() {
            self.su = other.su;
        }
        if other.umask.is_some() {
            self.umask = other.umask;
        }
        if other.defnonblock.is_some() {
            self.defnonblock = other.defnonblock;
        }
        if other.lockscreen.is_some() {
            self.lockscreen = other.lockscreen;
        }
        if other.partial.is_some() {
            self.partial = other.partial;
        }
        if other.writelock.is_some() {
            self.writelock = other.writelock;
        }
        if other.time.is_some() {
            self.time = other.time;
        }
        self.termcap_overrides.extend(other.termcap_overrides);
        self.attrcolor.extend(other.attrcolor);
        self.acl.extend(other.acl);
        self.bindings.extend(other.bindings);
        self.startup_windows.extend(other.startup_windows);
        if other.defhstatus.is_some() {
            self.defhstatus = other.defhstatus;
        }
        if other.defobuflimit.is_some() {
            self.defobuflimit = other.defobuflimit;
        }
        if other.cjkwidth.is_some() {
            self.cjkwidth = other.cjkwidth;
        }
        self.rendition.extend(other.rendition);
        self.unbind_keys.extend(other.unbind_keys);
        self.unbindkey_keys.extend(other.unbindkey_keys);
        if other.width.is_some() {
            self.width = other.width;
        }
        if other.debug.is_some() {
            self.debug = other.debug;
        }
        if other.login.is_some() {
            self.login = other.login;
        }
        if other.bufsize.is_some() {
            self.bufsize = other.bufsize;
        }
        self.layout.extend(other.layout);
        if other.title.is_some() {
            self.title = other.title;
        }
        if other.monitor.is_some() {
            self.monitor = other.monitor;
        }
        if other.stuff.is_some() {
            self.stuff = other.stuff;
        }
        self.eval_cmds.extend(other.eval_cmds);
        self.exec_cmds.extend(other.exec_cmds);
        self.at_cmds.extend(other.at_cmds);
        if other.copy.is_some() {
            self.copy = other.copy;
        }
        if other.paste.is_some() {
            self.paste = other.paste;
        }
        self.register.extend(other.register);
        self.readreg.extend(other.readreg);
        self.writereg.extend(other.writereg);
        if other.writebuf.is_some() {
            self.writebuf = other.writebuf;
        }
        if other.readbuf.is_some() {
            self.readbuf = other.readbuf;
        }
        if other.removebuf.is_some() {
            self.removebuf = other.removebuf;
        }
        if other.defkmap.is_some() {
            self.defkmap = other.defkmap;
        }
        if other.defcmnd.is_some() {
            self.defcmnd = other.defcmnd;
        }
        if other.deflist.is_some() {
            self.deflist = other.deflist;
        }
        if other.deftype.is_some() {
            self.deftype = other.deftype;
        }
        if other.defautoparam.is_some() {
            self.defautoparam = other.defautoparam;
        }
        if other.defpanposition.is_some() {
            self.defpanposition = other.defpanposition;
        }
        if other.focus.is_some() {
            self.focus = other.focus;
        }
        if other.dump.is_some() {
            self.dump = other.dump;
        }
        if other.deselect.is_some() {
            self.deselect = other.deselect;
        }
        if other.currwin.is_some() {
            self.currwin = other.currwin;
        }
        if other.ansi_partial.is_some() {
            self.ansi_partial = other.ansi_partial;
        }
        if other.autorefresh.is_some() {
            self.autorefresh = other.autorefresh;
        }
        if other.charset.is_some() {
            self.charset = other.charset;
        }
        if other.flow_cmd.is_some() {
            self.flow_cmd = other.flow_cmd;
        }
        if other.xon.is_some() {
            self.xon = other.xon;
        }
        if other.xoff.is_some() {
            self.xoff = other.xoff;
        }
        if other.colon.is_some() {
            self.colon = other.colon;
        }
        if other.kmap.is_some() {
            self.kmap = other.kmap;
        }
        if other.keybuf.is_some() {
            self.keybuf = other.keybuf;
        }
        if other.obufalloc.is_some() {
            self.obufalloc = other.obufalloc;
        }
        if other.obufcount.is_some() {
            self.obufcount = other.obufcount;
        }
        if other.obufwait.is_some() {
            self.obufwait = other.obufwait;
        }
        if other.dense.is_some() {
            self.dense = other.dense;
        }
        if other.mapdefault.is_some() {
            self.mapdefault = other.mapdefault;
        }
        if other.mapnext.is_some() {
            self.mapnext = other.mapnext;
        }
        self.pred.extend(other.pred);
        if other.clear_screen.is_some() {
            self.clear_screen = other.clear_screen;
        }
        if other.defbufflim.is_some() {
            self.defbufflim = other.defbufflim;
        }
        if other.hstatus.is_some() {
            self.hstatus = other.hstatus;
        }
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
    fn termcap_and_terminfo_stored_as_overrides() {
        let config = parse_config(
            b"termcap xterm 'AF=\\E[3%dm:AB=\\E[4%dm'\nterminfo xterm 'AF=\\E[3%dm'\n",
        )
        .expect("config parses");
        assert_eq!(config.termcap_overrides.len(), 2);
        assert_eq!(config.termcap_overrides[0].0, b"xterm");
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

    #[test]
    fn parses_new_config_commands() {
        let config = parse_config(
            b"activity \"%n has activity\"\nbell visible\nhardcopydir /tmp/hc\nlogtstamp after 60\ndeflogin on\nattrcolor b \"R\"\nattrcolor i \"+b\"\nallpartial on\nautofocus on\ndefcharset utf-8\nsilence 30\nsilencewait 15\nwrap on\nwrapsize 80\nbreaktype 250\ndefmousetrack on\nactivity \"%n@%t activity\"\nwindowlist on\nencoding utf-8\n",
        )
        .expect("config parses");

        assert_eq!(config.activity, Some(b"%n@%t activity".to_vec()));
        assert_eq!(config.bell_action, Some(b"visible".to_vec()));
        assert_eq!(config.hardcopydir, Some(b"/tmp/hc".to_vec()));
        assert_eq!(config.logtstamp, Some(b"after 60".to_vec()));
        assert_eq!(config.deflogin, Some(true));
        assert_eq!(config.attrcolor.len(), 2);
        assert_eq!(config.attrcolor[0], (b"b".to_vec(), Some(b"R".to_vec())));
        assert_eq!(config.attrcolor[1], (b"i".to_vec(), Some(b"+b".to_vec())));
        assert_eq!(config.allpartial, Some(true));
        assert_eq!(config.autofocus, Some(true));
        assert_eq!(config.defcharset, Some(b"utf-8".to_vec()));
        assert_eq!(config.silence, Some(b"30".to_vec()));
        assert_eq!(config.silencewait, Some(15));
        assert_eq!(config.wrap, Some(true));
        assert_eq!(config.wrapsize, Some(80));
        assert_eq!(config.breaktype, Some(250));
        assert_eq!(config.defmousetrack, Some(true));
        assert_eq!(config.windowlist, Some(true));
        assert_eq!(config.encoding, Some(b"utf-8".to_vec()));
    }

    #[test]
    fn accepted_commands_parse_silently() {
        let cmds = b"echo ready\nsleep 1\nnumber 5\nsort\nprocess group\nversion\nnext\nprev\nother\ndetach\nkill\nquit\nhardcopy\ndigraph\nfit\nonly\nremove\nreset\nkey\nverbose\ndinfo\n";
        let config = parse_config(cmds).expect("accepted commands parse");
        // Should parse successfully with default config.
        assert_eq!(config.shell, None);
    }

    // ── New tests for bindkey ──

    #[test]
    fn test_parse_bindkey() {
        let config = parse_config(b"bindkey -k k1 select 1\n").expect("bindkey parses");
        assert_eq!(config.bindings.len(), 1);
        assert_eq!(
            config.bindings[0].command,
            vec![b"select".to_vec(), b"1".to_vec()]
        );
    }

    // ── Screen window creation tests ──

    #[test]
    fn test_parse_screen_command() {
        // screen -t title 1 sh: -t is not special-cased, stored as program
        let config = parse_config(b"screen -t title 1 sh\n").expect("screen command parses");
        assert_eq!(config.startup_windows.len(), 1);
    }

    #[test]
    fn test_parse_screen_without_number() {
        let config = parse_config(b"screen sh -c \"top\"\n").expect("screen command parses");
        assert_eq!(config.startup_windows.len(), 1);
        assert_eq!(config.startup_windows[0].program, Some(b"sh".to_vec()));
        assert_eq!(
            config.startup_windows[0].args,
            vec![b"-c".to_vec(), b"top".to_vec()]
        );
    }

    // ── Blank lines ──

    #[test]
    fn test_parse_blank_lines() {
        let config = parse_config(b"\n\nshell /bin/sh\n\n\n").expect("blank lines ignored");
        assert_eq!(config.shell, Some(b"/bin/sh".to_vec()));
    }

    // ── Environment variable commands ──

    #[test]
    fn test_parse_setenv() {
        let config = parse_config(b"setenv PATH /usr/local/bin:$PATH\n").expect("setenv parses");
        assert_eq!(config.setenv.len(), 1);
        assert_eq!(config.setenv[0].0, b"PATH");
        assert_eq!(config.setenv[0].1, b"/usr/local/bin:$PATH");
    }

    #[test]
    fn test_parse_unsetenv() {
        let config = parse_config(b"unsetenv TEMP\n").expect("unsetenv parses");
        assert_eq!(config.unsetenv.len(), 1);
        assert_eq!(config.unsetenv[0], b"TEMP");
    }

    // ── Backtick command ──

    #[test]
    fn test_parse_backtick() {
        let config = parse_config(b"backtick 0 0 0 echo hello\n").expect("backtick parses");
        assert_eq!(config.backtick.len(), 1);
        assert_eq!(config.backtick[0].id, 0);
        assert_eq!(config.backtick[0].command, b"echo hello");
    }

    // ── ACL commands ──

    #[test]
    fn test_parse_acl() {
        let config = parse_config(b"acladd user rwx\n").expect("acladd parses");
        assert_eq!(config.acl.len(), 1);
        assert_eq!(config.acl[0].username, b"user");
        assert_eq!(config.acl[0].permissions, b"rwx");
    }

    #[test]
    fn test_parse_invalid_number() {
        let err = parse_config(b"defscrollback not_a_number\n").expect_err("should error");
        assert!(matches!(err.kind, ConfigErrorKind::InvalidArgument { .. }));
    }

    // ── Source recursion limit ──

    #[test]
    fn test_source_recursion_limit() {
        let directory = temp_directory("recursion");
        std::fs::create_dir(&directory).expect("create temp dir");
        // Create chain of 20 files that source each other (exceeds limit of 16)
        for i in 0..20 {
            let path = directory.join(format!("{}.screenrc", i));
            let next = if i < 19 {
                format!("{}.screenrc", i + 1)
            } else {
                "0.screenrc".to_string()
            };
            std::fs::write(&path, format!("source {next}\n").as_bytes()).expect("write config");
        }
        let root = directory.join("0.screenrc");
        let err = parse_config_file(&root).expect_err("should hit recursion limit");
        assert!(
            matches!(err.kind, ConfigErrorKind::SourceRecursionLimit { .. }),
            "expected SourceRecursionLimit, got {:?}",
            err.kind
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    // ── Escape parsing variants ──

    #[test]
    fn test_escape_parsing() {
        // ^A -> Ctrl-A (0x01)
        let config = parse_config(b"escape ^Aa\n").expect("escape ^Aa");
        assert_eq!(config.escape, Some(vec![0x01, b'a']));

        // ^[ -> Escape (0x1b)
        let config = parse_config(b"escape ^[x\n").expect("escape ^[x");
        assert_eq!(config.escape, Some(vec![0x1b, b'x']));

        // ^? -> DEL (0x7f)
        let config = parse_config(b"escape ^?x\n").expect("escape ^?x");
        assert_eq!(config.escape, Some(vec![0x7f, b'x']));
    }

    // ── Complex full-featured .screenrc ──

    #[test]
    fn test_parse_complex_screenrc() {
        let config = parse_config(
            b"# Full featured screenrc\n\
              shell /bin/bash\n\
              term screen-256color\n\
              defscrollback 5000\n\
              startup_message off\n\
              hardstatus alwayslastline \"%{= kc}%{+b}%n %t%? %u%?%=\"\n\
              caption always \"%{= bb}%n %t %h\"\n\
              bind ^d detach\n\
              bind ^k kill\n\
              setenv PATH /usr/local/bin:$PATH\n\
              setenv EDITOR vim\n\
              unsetenv TEMP\n\
              log on\n\
              vbell on\n\
              defmonitor on\n\
              backtick 0 0 0 loadavg\n\
              acladd admin rw\n\
              screen 0\n\
              select 0\n",
        )
        .expect("complex screenrc parses");

        assert_eq!(config.shell, Some(b"/bin/bash".to_vec()));
        assert_eq!(config.term, Some(b"screen-256color".to_vec()));
        assert_eq!(config.defscrollback, Some(5000));
        assert_eq!(config.startup_message, Some(false));
        assert!(config.hardstatus.is_some());
        assert!(config.caption.is_some());
        assert_eq!(config.bindings.len(), 2);
        assert_eq!(config.setenv.len(), 2);
        assert_eq!(config.unsetenv.len(), 1);
        assert_eq!(config.backtick.len(), 1);
        assert_eq!(config.acl.len(), 1);
        assert_eq!(config.select, Some(0));
    }

    // ── parse_config_file with temp file ──

    #[test]
    fn test_parse_config_file() {
        let directory = temp_directory("cfgfile");
        std::fs::create_dir(&directory).expect("create temp dir");
        let file = directory.join(".screenrc");
        std::fs::write(
            &file,
            b"shell /tmp/sh\nterm xterm-256color\nchdir /tmp\nlog on\n",
        )
        .expect("write config file");

        let config = parse_config_file(&file).expect("config file parses");
        assert_eq!(config.shell, Some(b"/tmp/sh".to_vec()));
        assert_eq!(config.term, Some(b"xterm-256color".to_vec()));
        assert_eq!(config.chdir, Some(b"/tmp".to_vec()));
        assert_eq!(config.logging, Some(true));

        let _ = std::fs::remove_dir_all(directory);
    }

    fn temp_directory(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("sc-cfg-{name}-{}-{nanos}", std::process::id()))
    }
}
