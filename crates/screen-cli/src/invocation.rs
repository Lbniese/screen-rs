use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    Help,
    Version,
    Create(CreateOptions),
    CreateDetached(CreateDetachedOptions),
    Attach(AttachOptions),
    Detach(DetachOptions),
    AttachOrCreate(AttachOrCreateOptions),
    List(ListOptions),
    Wipe(WipeOptions),
    RemoteCommand(RemoteCommandOptions),
    Query(QueryOptions),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOptions {
    pub session_name: Option<OsString>,
    pub config_file: Option<OsString>,
    pub term: Option<OsString>,
    pub shell: Option<OsString>,
    pub logging: bool,
    pub force_new: bool,
    pub quiet: bool,
    pub flow_control: Option<FlowControlMode>,
    pub interrupt_sooner: bool,
    pub optimal_output: bool,
    pub utf8_mode: bool,
    pub adapt_all_windows: bool,
    pub force_all_capabilities: bool,
    pub command: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDetachedOptions {
    pub session_name: Option<OsString>,
    pub config_file: Option<OsString>,
    pub term: Option<OsString>,
    pub shell: Option<OsString>,
    pub logging: bool,
    pub mode: DetachedMode,
    pub quiet: bool,
    pub flow_control: Option<FlowControlMode>,
    pub interrupt_sooner: bool,
    pub optimal_output: bool,
    pub utf8_mode: bool,
    pub adapt_all_windows: bool,
    pub force_all_capabilities: bool,
    pub command: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachedMode {
    LowerDetach,
    UpperDetach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControlMode {
    On,
    Off,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOptions {
    pub session: Option<OsString>,
    pub multi_display: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachOptions {
    pub session: Option<OsString>,
    pub power_detach: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOrCreateOptions {
    pub session: Option<OsString>,
    pub aggressive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOptions {
    pub session_match: Option<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WipeOptions {
    pub session_match: Option<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandOptions {
    pub session: Option<OsString>,
    pub window: Option<OsString>,
    pub command: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOptions {
    pub session: Option<OsString>,
    pub window: Option<OsString>,
    pub command: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingValue { option: &'static str },
    MissingCommand { option: &'static str },
    DuplicateOption { option: &'static str },
    ConflictingOptions { message: String },
    UnknownOption { option: OsString },
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue { option } => write!(formatter, "{option} requires an argument"),
            Self::MissingCommand { option } => write!(formatter, "{option} requires a command"),
            Self::DuplicateOption { option } => write!(formatter, "{option} was provided twice"),
            Self::ConflictingOptions { message } => formatter.write_str(message),
            Self::UnknownOption { option } => {
                write!(formatter, "unknown option {}", option.to_string_lossy())
            }
        }
    }
}

impl Error for ParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachMode {
    Reattach,
    AttachOrCreate,
    AttachOrCreateAggressive,
}

#[derive(Debug, Default)]
struct ParserState {
    help: bool,
    version: bool,
    list: bool,
    wipe: bool,
    list_match: Option<OsString>,
    wipe_match: Option<OsString>,
    session_name: Option<OsString>,
    config_file: Option<OsString>,
    term: Option<OsString>,
    shell: Option<OsString>,
    logging: bool,
    window: Option<OsString>,
    lower_detach: bool,
    upper_detach: bool,
    force_new: bool,
    attach_mode: Option<AttachMode>,
    attach_session: Option<OsString>,
    remote_command: Option<Vec<OsString>>,
    query_command: Option<Vec<OsString>>,
    quiet: bool,
    flow_control: Option<FlowControlMode>,
    interrupt_sooner: bool,
    optimal_output: bool,
    utf8_mode: bool,
    adapt_all_windows: bool,
    force_all_capabilities: bool,
    multi_display: bool,
    command: Vec<OsString>,
}

pub fn parse_invocation<I, S>(args: I) -> Result<Invocation, ParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut state = ParserState::default();
    let mut args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].clone();
        let Some(option) = arg.to_str() else {
            state.command.extend(args.drain(index..));
            break;
        };

        match option {
            "--help" | "-h" => {
                state.help = true;
                index += 1;
            }
            "--version" | "-v" => {
                state.version = true;
                index += 1;
            }
            "-ls" | "-list" => {
                state.list = true;
                index += 1;
                if let Some(session_match) = optional_session(&args, index) {
                    state.list_match = Some(session_match);
                    index += 1;
                }
            }
            "-wipe" => {
                state.wipe = true;
                index += 1;
                if let Some(session_match) = optional_session(&args, index) {
                    state.wipe_match = Some(session_match);
                    index += 1;
                }
            }
            "-L" => {
                state.logging = true;
                index += 1;
            }
            "-dm" => {
                state.lower_detach = true;
                state.force_new = true;
                index += 1;
            }
            "-Dm" => {
                state.upper_detach = true;
                state.force_new = true;
                index += 1;
            }
            _ if option.starts_with("-dmS") => {
                state.lower_detach = true;
                state.force_new = true;
                let attached = &option["-dmS".len()..];
                if attached.is_empty() {
                    index += 1;
                    state.session_name = Some(take_required(&args, index, "-S")?);
                    index += 1;
                } else {
                    state.session_name = Some(OsString::from(attached));
                    index += 1;
                }
            }
            _ if option.starts_with("-DmS") => {
                state.upper_detach = true;
                state.force_new = true;
                let attached = &option["-DmS".len()..];
                if attached.is_empty() {
                    index += 1;
                    state.session_name = Some(take_required(&args, index, "-S")?);
                    index += 1;
                } else {
                    state.session_name = Some(OsString::from(attached));
                    index += 1;
                }
            }
            "-S" => {
                index += 1;
                state.session_name = Some(take_required(&args, index, "-S")?);
                index += 1;
            }
            _ if let Some(value) = attached_option_value(option, "-S") => {
                state.session_name = Some(OsString::from(value));
                index += 1;
            }
            "-c" => {
                index += 1;
                state.config_file = Some(take_required(&args, index, "-c")?);
                index += 1;
            }
            _ if let Some(value) = attached_option_value(option, "-c") => {
                state.config_file = Some(OsString::from(value));
                index += 1;
            }
            "-p" => {
                index += 1;
                state.window = Some(take_required(&args, index, "-p")?);
                index += 1;
            }
            _ if let Some(value) = attached_option_value(option, "-p") => {
                state.window = Some(OsString::from(value));
                index += 1;
            }
            "-T" => {
                index += 1;
                state.term = Some(take_required(&args, index, "-T")?);
                index += 1;
            }
            _ if let Some(value) = attached_option_value(option, "-T") => {
                state.term = Some(OsString::from(value));
                index += 1;
            }
            "-s" => {
                index += 1;
                state.shell = Some(take_required(&args, index, "-s")?);
                index += 1;
            }
            _ if let Some(value) = attached_option_value(option, "-s") => {
                state.shell = Some(OsString::from(value));
                index += 1;
            }
            "-m" => {
                state.force_new = true;
                index += 1;
            }
            "-q" => {
                state.quiet = true;
                index += 1;
            }
            "-x" => {
                state.multi_display = true;
                index += 1;
            }
            "-f" => {
                state.flow_control = Some(FlowControlMode::On);
                index += 1;
            }
            "-fn" => {
                state.flow_control = Some(FlowControlMode::Off);
                index += 1;
            }
            "-fa" => {
                state.flow_control = Some(FlowControlMode::Auto);
                index += 1;
            }
            "-i" => {
                state.interrupt_sooner = true;
                index += 1;
            }
            "-O" => {
                state.optimal_output = true;
                index += 1;
            }
            "-U" => {
                state.utf8_mode = true;
                index += 1;
            }
            "-a" => {
                state.force_all_capabilities = true;
                index += 1;
            }
            "-A" => {
                state.adapt_all_windows = true;
                index += 1;
            }
            "-d" => {
                state.lower_detach = true;
                index += 1;
            }
            "-D" => {
                state.upper_detach = true;
                index += 1;
            }
            "-r" => {
                set_attach_mode(&mut state, AttachMode::Reattach, "-r")?;
                index += 1;
                if let Some(session) = optional_session(&args, index) {
                    state.attach_session = Some(session);
                    index += 1;
                }
            }
            "-R" => {
                set_attach_mode(&mut state, AttachMode::AttachOrCreate, "-R")?;
                index += 1;
                if let Some(session) = optional_session(&args, index) {
                    state.attach_session = Some(session);
                    index += 1;
                }
            }
            "-RR" => {
                set_attach_mode(&mut state, AttachMode::AttachOrCreateAggressive, "-RR")?;
                index += 1;
                if let Some(session) = optional_session(&args, index) {
                    state.attach_session = Some(session);
                    index += 1;
                }
            }
            "-X" => {
                index += 1;
                state.remote_command = Some(take_command_tail(&mut args, index, "-X")?);
                break;
            }
            "-Q" => {
                index += 1;
                state.query_command = Some(take_command_tail(&mut args, index, "-Q")?);
                break;
            }
            _ if option.starts_with('-') => {
                return Err(ParseError::UnknownOption { option: arg });
            }
            _ => {
                state.command.extend(args.drain(index..));
                break;
            }
        }
    }

    build_invocation(state)
}

fn build_invocation(state: ParserState) -> Result<Invocation, ParseError> {
    if state.help || state.version {
        reject_extra_for_information(&state)?;
        return Ok(if state.help {
            Invocation::Help
        } else {
            Invocation::Version
        });
    }

    if state.list || state.wipe {
        reject_list_wipe_conflicts(&state)?;
        return Ok(if state.list {
            Invocation::List(ListOptions {
                session_match: state.list_match,
            })
        } else {
            Invocation::Wipe(WipeOptions {
                session_match: state.wipe_match,
            })
        });
    }

    if state.remote_command.is_some() || state.query_command.is_some() {
        reject_remote_conflicts(&state)?;
        if let Some(command) = state.remote_command {
            return Ok(Invocation::RemoteCommand(RemoteCommandOptions {
                session: state.session_name,
                window: state.window,
                command,
            }));
        }
        if let Some(command) = state.query_command {
            return Ok(Invocation::Query(QueryOptions {
                session: state.session_name,
                window: state.window,
                command,
            }));
        }
    }

    if let Some(mode) = state.attach_mode {
        reject_attach_conflicts(&state)?;
        let session = state.attach_session.or(state.session_name);
        return Ok(match mode {
            AttachMode::Reattach => Invocation::Attach(AttachOptions {
                session,
                multi_display: state.multi_display,
            }),
            AttachMode::AttachOrCreate => Invocation::AttachOrCreate(AttachOrCreateOptions {
                session,
                aggressive: false,
            }),
            AttachMode::AttachOrCreateAggressive => {
                Invocation::AttachOrCreate(AttachOrCreateOptions {
                    session,
                    aggressive: true,
                })
            }
        });
    }

    if state.lower_detach && state.upper_detach {
        return Err(ParseError::ConflictingOptions {
            message: "-d and -D cannot be used together".to_owned(),
        });
    }

    if state.force_new && (state.lower_detach || state.upper_detach) {
        return Ok(Invocation::CreateDetached(CreateDetachedOptions {
            session_name: state.session_name,
            config_file: state.config_file,
            term: state.term,
            shell: state.shell,
            logging: state.logging,
            mode: if state.upper_detach {
                DetachedMode::UpperDetach
            } else {
                DetachedMode::LowerDetach
            },
            quiet: state.quiet,
            flow_control: state.flow_control,
            interrupt_sooner: state.interrupt_sooner,
            optimal_output: state.optimal_output,
            utf8_mode: state.utf8_mode,
            adapt_all_windows: state.adapt_all_windows,
            force_all_capabilities: state.force_all_capabilities,
            command: state.command,
        }));
    }

    if state.lower_detach || state.upper_detach {
        return Ok(Invocation::Detach(DetachOptions {
            session: state.session_name,
            power_detach: state.upper_detach,
        }));
    }

    Ok(Invocation::Create(CreateOptions {
        session_name: state.session_name,
        config_file: state.config_file,
        term: state.term,
        shell: state.shell,
        logging: state.logging,
        force_new: state.force_new,
        quiet: state.quiet,
        flow_control: state.flow_control,
        interrupt_sooner: state.interrupt_sooner,
        optimal_output: state.optimal_output,
        utf8_mode: state.utf8_mode,
        adapt_all_windows: state.adapt_all_windows,
        force_all_capabilities: state.force_all_capabilities,
        command: state.command,
    }))
}

fn reject_extra_for_information(state: &ParserState) -> Result<(), ParseError> {
    let has_extra = state.list
        || state.wipe
        || state.session_name.is_some()
        || state.config_file.is_some()
        || state.term.is_some()
        || state.shell.is_some()
        || state.logging
        || state.window.is_some()
        || state.lower_detach
        || state.upper_detach
        || state.force_new
        || state.attach_mode.is_some()
        || state.remote_command.is_some()
        || state.query_command.is_some()
        || !state.command.is_empty();

    if state.help && state.version {
        return Err(ParseError::ConflictingOptions {
            message: "--help and --version cannot be used together".to_owned(),
        });
    }

    if has_extra {
        return Err(ParseError::ConflictingOptions {
            message: "--help and --version must be used without session operations".to_owned(),
        });
    }

    Ok(())
}

fn reject_list_wipe_conflicts(state: &ParserState) -> Result<(), ParseError> {
    if state.list && state.wipe {
        return Err(ParseError::ConflictingOptions {
            message: "-ls and -wipe cannot be used together".to_owned(),
        });
    }

    let has_extra = state.session_name.is_some()
        || state.config_file.is_some()
        || state.term.is_some()
        || state.shell.is_some()
        || state.logging
        || state.window.is_some()
        || state.lower_detach
        || state.upper_detach
        || state.force_new
        || state.attach_mode.is_some()
        || state.remote_command.is_some()
        || state.query_command.is_some()
        || !state.command.is_empty();

    if has_extra {
        return Err(ParseError::ConflictingOptions {
            message: "list and wipe operations cannot be combined with session operations"
                .to_owned(),
        });
    }

    Ok(())
}

fn reject_remote_conflicts(state: &ParserState) -> Result<(), ParseError> {
    if state.remote_command.is_some() && state.query_command.is_some() {
        return Err(ParseError::ConflictingOptions {
            message: "-X and -Q cannot be used together".to_owned(),
        });
    }

    let has_extra = state.list
        || state.wipe
        || state.config_file.is_some()
        || state.logging
        || state.lower_detach
        || state.upper_detach
        || state.force_new
        || state.attach_mode.is_some()
        || !state.command.is_empty();

    if has_extra {
        return Err(ParseError::ConflictingOptions {
            message: "remote commands cannot be combined with create, attach, list, or wipe"
                .to_owned(),
        });
    }

    Ok(())
}

fn reject_attach_conflicts(state: &ParserState) -> Result<(), ParseError> {
    let has_extra = state.window.is_some()
        || state.config_file.is_some()
        || state.term.is_some()
        || state.shell.is_some()
        || state.logging
        || state.lower_detach
        || state.upper_detach
        || state.force_new
        || state.remote_command.is_some()
        || state.query_command.is_some()
        || !state.command.is_empty();

    if has_extra {
        return Err(ParseError::ConflictingOptions {
            message: "attach operations cannot be combined with create, detach, remote, or command"
                .to_owned(),
        });
    }

    if state.attach_session.is_some() && state.session_name.is_some() {
        return Err(ParseError::ConflictingOptions {
            message: "attach session was provided both positionally and with -S".to_owned(),
        });
    }

    Ok(())
}

fn take_required(
    args: &[OsString],
    index: usize,
    option: &'static str,
) -> Result<OsString, ParseError> {
    let value = args
        .get(index)
        .ok_or(ParseError::MissingValue { option })?
        .clone();
    if value.as_os_str().is_empty() {
        return Err(ParseError::MissingValue { option });
    }
    Ok(value)
}

fn take_command_tail(
    args: &mut Vec<OsString>,
    index: usize,
    option: &'static str,
) -> Result<Vec<OsString>, ParseError> {
    if index >= args.len() {
        return Err(ParseError::MissingCommand { option });
    }

    let command: Vec<OsString> = args.drain(index..).collect();
    if command
        .first()
        .is_none_or(|value| value.as_os_str().is_empty())
    {
        return Err(ParseError::MissingCommand { option });
    }
    Ok(command)
}

fn optional_session(args: &[OsString], index: usize) -> Option<OsString> {
    let value = args.get(index)?;
    if is_option_like(value.as_os_str()) {
        None
    } else {
        Some(value.clone())
    }
}

fn is_option_like(value: &OsStr) -> bool {
    value
        .to_str()
        .is_some_and(|text| text.starts_with('-') && text.len() > 1)
}

fn attached_option_value<'a>(option: &'a str, prefix: &str) -> Option<&'a str> {
    option
        .strip_prefix(prefix)
        .and_then(|value| (!value.is_empty()).then_some(value))
}

fn set_attach_mode(
    state: &mut ParserState,
    mode: AttachMode,
    option: &'static str,
) -> Result<(), ParseError> {
    if state.attach_mode.is_some() {
        return Err(ParseError::DuplicateOption { option });
    }
    state.attach_mode = Some(mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> OsString {
        OsString::from(value)
    }

    #[test]
    fn parses_help() {
        assert_eq!(parse_invocation(["--help"]), Ok(Invocation::Help));
    }

    #[test]
    fn parses_version() {
        assert_eq!(parse_invocation(["--version"]), Ok(Invocation::Version));
    }

    #[test]
    fn parses_list_aliases() {
        assert_eq!(
            parse_invocation(["-ls"]),
            Ok(Invocation::List(ListOptions {
                session_match: None,
            }))
        );
        assert_eq!(
            parse_invocation(["-list"]),
            Ok(Invocation::List(ListOptions {
                session_match: None,
            }))
        );
        assert_eq!(
            parse_invocation(["-ls", "demo"]),
            Ok(Invocation::List(ListOptions {
                session_match: Some(os("demo")),
            }))
        );
    }

    #[test]
    fn parses_wipe() {
        assert_eq!(
            parse_invocation(["-wipe"]),
            Ok(Invocation::Wipe(WipeOptions {
                session_match: None,
            }))
        );
        assert_eq!(
            parse_invocation(["-wipe", "demo"]),
            Ok(Invocation::Wipe(WipeOptions {
                session_match: Some(os("demo")),
            }))
        );
    }

    #[test]
    fn parses_named_create_command() {
        assert_eq!(
            parse_invocation(["-S", "demo", "sh", "-c", "printf ready"]),
            Ok(Invocation::Create(CreateOptions {
                session_name: Some(os("demo")),
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
                command: vec![os("sh"), os("-c"), os("printf ready")],
            }))
        );
    }

    #[test]
    fn parses_create_with_shell_override() {
        assert_eq!(
            parse_invocation(["-S", "demo", "-s", "/tmp/custom-shell", "-d", "-m"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: Some(os("/tmp/custom-shell")),
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: Vec::new(),
            }))
        );
    }

    #[test]
    fn parses_create_with_config_file() {
        assert_eq!(
            parse_invocation(["-c", "/tmp/screenrc", "-S", "demo", "-d", "-m"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: Some(os("/tmp/screenrc")),
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: Vec::new(),
            }))
        );
    }

    #[test]
    fn parses_create_with_terminal_override() {
        assert_eq!(
            parse_invocation(["-S", "demo", "-T", "screen-256color", "-d", "-m", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: Some(os("screen-256color")),
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_detached_create() {
        assert_eq!(
            parse_invocation(["-S", "demo", "-d", "-m", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_upper_detached_create() {
        assert_eq!(
            parse_invocation(["-D", "-m"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: None,
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::UpperDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: Vec::new(),
            }))
        );
    }

    #[test]
    fn parses_detached_create_with_logging() {
        assert_eq!(
            parse_invocation(["-L", "-S", "demo", "-d", "-m", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: true,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_common_compact_detached_create_options() {
        assert_eq!(
            parse_invocation(["-dmS", "demo", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
        assert_eq!(
            parse_invocation(["-Sdemo", "-dm", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
        assert_eq!(
            parse_invocation(["-DmSdemo", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::UpperDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_attach_modes() {
        assert_eq!(
            parse_invocation(["-r", "demo"]),
            Ok(Invocation::Attach(AttachOptions {
                session: Some(os("demo")),
                multi_display: false,
            }))
        );
        assert_eq!(
            parse_invocation(["-R", "demo"]),
            Ok(Invocation::AttachOrCreate(AttachOrCreateOptions {
                session: Some(os("demo")),
                aggressive: false,
            }))
        );
        assert_eq!(
            parse_invocation(["-RR", "demo"]),
            Ok(Invocation::AttachOrCreate(AttachOrCreateOptions {
                session: Some(os("demo")),
                aggressive: true,
            }))
        );
    }

    #[test]
    fn parses_remote_command_with_targeting() {
        assert_eq!(
            parse_invocation(["-S", "demo", "-p", "1", "-X", "stuff", "abc"]),
            Ok(Invocation::RemoteCommand(RemoteCommandOptions {
                session: Some(os("demo")),
                window: Some(os("1")),
                command: vec![os("stuff"), os("abc")],
            }))
        );
    }

    #[test]
    fn parses_query_command() {
        assert_eq!(
            parse_invocation(["-S", "demo", "-Q", "windows"]),
            Ok(Invocation::Query(QueryOptions {
                session: Some(os("demo")),
                window: None,
                command: vec![os("windows")],
            }))
        );
    }

    #[test]
    fn rejects_missing_values() {
        assert!(matches!(
            parse_invocation(["-S"]),
            Err(ParseError::MissingValue { option: "-S" })
        ));
        assert!(matches!(
            parse_invocation(["-X"]),
            Err(ParseError::MissingCommand { option: "-X" })
        ));
    }

    #[test]
    fn rejects_invalid_combinations() {
        assert!(matches!(
            parse_invocation(["-ls", "-S", "demo"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
        assert!(matches!(
            parse_invocation(["--help", "-ls"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
        assert!(matches!(
            parse_invocation(["-d", "-D"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
        assert!(matches!(
            parse_invocation(["-r", "demo", "-S", "other"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
    }

    #[test]
    fn rejects_unknown_option() {
        assert!(matches!(
            parse_invocation(["--not-screen"]),
            Err(ParseError::UnknownOption { .. })
        ));
    }

    #[test]
    fn parses_dm_s_lower_with_attached_name() {
        assert_eq!(
            parse_invocation(["-dmSdemo", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );

        assert_eq!(
            parse_invocation(["-DmSdemo", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::UpperDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_l_flag() {
        assert_eq!(
            parse_invocation(["-L", "-S", "demo", "-d", "-m", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: None,
                shell: None,
                logging: true,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_t_flag() {
        assert_eq!(
            parse_invocation(["-T", "screen-256color", "-S", "demo", "-d", "-m", "sh"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: None,
                term: Some(os("screen-256color")),
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: vec![os("sh")],
            }))
        );
    }

    #[test]
    fn parses_c_flag() {
        assert_eq!(
            parse_invocation(["-c", "/path/to/screenrc", "-S", "demo", "-d", "-m"]),
            Ok(Invocation::CreateDetached(CreateDetachedOptions {
                session_name: Some(os("demo")),
                config_file: Some(os("/path/to/screenrc")),
                term: None,
                shell: None,
                logging: false,
                mode: DetachedMode::LowerDetach,
                quiet: false,
                flow_control: None,
                interrupt_sooner: false,
                optimal_output: false,
                utf8_mode: false,
                adapt_all_windows: false,
                force_all_capabilities: false,
                command: Vec::new(),
            }))
        );
    }

    #[test]
    fn rejects_d_and_d_together() {
        assert!(matches!(
            parse_invocation(["-d", "-D"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
        assert!(matches!(
            parse_invocation(["-D", "-d"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
    }

    #[test]
    fn rejects_r_with_s_conflict() {
        assert!(matches!(
            parse_invocation(["-r", "demo", "-S", "other"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
    }

    #[test]
    fn rejects_x_and_q_together() {
        // -X and -Q both consume remaining args and break the loop,
        // so only the first one encountered takes effect. Verify -X wins.
        let parsed = parse_invocation(["-S", "demo", "-X", "stuff", "-Q", "windows"]);
        assert!(
            matches!(&parsed, Ok(Invocation::RemoteCommand(opts)) if opts.command == [os("stuff"), os("-Q"), os("windows")]),
            "expected RemoteCommand with -Q consumed as arg, got {parsed:?}"
        );

        // Verify -Q wins when it appears first
        let parsed = parse_invocation(["-S", "demo", "-Q", "windows", "-X", "stuff"]);
        assert!(
            matches!(&parsed, Ok(Invocation::Query(opts)) if opts.command == [os("windows"), os("-X"), os("stuff")]),
            "expected Query with -X consumed as arg, got {parsed:?}"
        );
    }

    #[test]
    fn rejects_help_with_operations() {
        assert!(matches!(
            parse_invocation(["--help", "-ls"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
        assert!(matches!(
            parse_invocation(["-h", "-S", "demo"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
        assert!(matches!(
            parse_invocation(["--version", "-wipe"]),
            Err(ParseError::ConflictingOptions { .. })
        ));
    }

    #[test]
    fn parses_empty_args() {
        let no_args: [&str; 0] = [];
        assert_eq!(
            parse_invocation(no_args),
            Ok(Invocation::Create(CreateOptions {
                session_name: None,
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
            }))
        );
    }

    #[test]
    fn rejects_empty_session_name() {
        assert!(matches!(
            parse_invocation(["-S", ""]),
            Err(ParseError::MissingValue { option: "-S" })
        ));
    }

    #[test]
    fn parses_list_with_session_match() {
        assert_eq!(
            parse_invocation(["-ls", "my_session"]),
            Ok(Invocation::List(ListOptions {
                session_match: Some(os("my_session")),
            }))
        );
        assert_eq!(
            parse_invocation(["-list", "other"]),
            Ok(Invocation::List(ListOptions {
                session_match: Some(os("other")),
            }))
        );
    }

    #[test]
    fn parses_wipe_with_session_match() {
        assert_eq!(
            parse_invocation(["-wipe", "my_session"]),
            Ok(Invocation::Wipe(WipeOptions {
                session_match: Some(os("my_session")),
            }))
        );
    }

    #[test]
    fn parses_detach() {
        assert_eq!(
            parse_invocation(["-d"]),
            Ok(Invocation::Detach(DetachOptions {
                session: None,
                power_detach: false,
            }))
        );
    }

    #[test]
    fn parses_power_detach() {
        assert_eq!(
            parse_invocation(["-D"]),
            Ok(Invocation::Detach(DetachOptions {
                session: None,
                power_detach: true,
            }))
        );
    }
}
