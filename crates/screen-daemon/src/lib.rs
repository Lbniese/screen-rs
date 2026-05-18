#![deny(unsafe_code)]

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use screen_protocol::{Message, ProtocolError, WindowInfoMsg};
use screen_pty::{PtyCommand, PtyError, PtyProcess, PtySize};
use screen_terminal::{Dimensions, TerminalState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Starting,
    Listening,
    ShuttingDown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub client_timeout: Duration,
}

impl DaemonConfig {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            client_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonReport {
    pub clients_served: usize,
}

pub fn run_until_shutdown(config: DaemonConfig) -> Result<DaemonReport, DaemonError> {
    ensure_parent_exists(&config.socket_path)?;
    reject_existing_socket_path(&config.socket_path)?;

    let listener = UnixListener::bind(&config.socket_path).map_err(|source| DaemonError::Bind {
        path: config.socket_path.clone(),
        source,
    })?;
    let _cleanup = SocketCleanup::new(config.socket_path.clone());
    let mut clients_served = 0;

    loop {
        let (mut stream, _address) = listener.accept().map_err(DaemonError::Accept)?;
        configure_client_timeouts(&stream, config.client_timeout)?;

        clients_served += 1;
        if handle_client(&mut stream)? == ClientOutcome::Shutdown {
            break;
        }
    }

    Ok(DaemonReport { clients_served })
}

#[derive(Debug, Clone)]
pub struct StartupWindow {
    pub title: Option<Vec<u8>>,
    pub program: Option<OsString>,
    pub args: Vec<OsString>,
    pub number: Option<u32>,
    pub working_directory: Option<OsString>,
    pub stuff: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct DaemonBacktick {
    pub id: u16,
    pub perpetual: bool,
    pub refresh_secs: Option<u32>,
    pub command: OsString,
}

#[derive(Debug, Clone)]
pub struct PtySessionConfig {
    pub socket_path: PathBuf,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub size: PtySize,
    pub terminal: OsString,
    pub log_path: Option<PathBuf>,
    pub working_directory: Option<PathBuf>,
    pub client_timeout: Duration,
    pub output_buffer_limit: usize,
    pub hardstatus_format: Option<Vec<u8>>,
    pub scrollback_limit: Option<u32>,
    /// Per-new-window defaults from config.
    pub default_monitor: Option<bool>,
    pub default_flow: Option<bool>,
    pub default_wrap: Option<bool>,
    pub default_silence: Option<u16>,
    pub auto_nuke: Option<bool>,
    /// Additional windows to create at session startup (from .screenrc).
    pub startup_windows: Vec<StartupWindow>,
    // ── New config fields ──
    /// Search case sensitivity.
    pub ignorecase: Option<bool>,
    /// Compact empty lines in scrollback.
    pub compacthist: Option<bool>,
    /// File for exchange buffer.
    pub bufferfile: Option<OsString>,
    /// Key sequences for copy mode marks.
    pub markkeys: Option<Vec<u8>>,
    /// Visual bell.
    pub vbell: Option<bool>,
    /// Visual bell message.
    pub vbell_msg: Option<Vec<u8>>,
    /// Audible bell message.
    pub bell_msg: Option<Vec<u8>>,
    /// Auto-detach on hangup.
    pub autodetach: Option<bool>,
    /// Per-window scrollback size.
    pub scrollback: Option<u32>,
    /// Message display time (seconds).
    pub msgwait: Option<u32>,
    /// Background color erase.
    pub bce: Option<bool>,
    /// Default UTF-8 mode.
    pub defutf8: Option<bool>,
    /// Default character encoding.
    pub defencoding: Option<OsString>,
    /// Slow paste delay (ms).
    pub slowpaste: Option<u32>,
    /// Session name for reattach.
    pub sessionname: Option<OsString>,
    /// Maximum number of windows.
    pub maxwin: Option<u32>,
    /// CR/LF mode (autocr).
    pub crlf: Option<bool>,
    /// Hardcopy print command.
    pub printcmd: Option<OsString>,
    /// Hardcopy append mode.
    pub hardcopy_append: Option<bool>,
    /// Non-blocking I/O mode.
    pub nonblock: Option<bool>,
    /// Zmodem catch.
    pub zmodem: Option<bool>,
    /// Wall message.
    pub wall: Option<Vec<u8>>,
    /// Backtick commands.
    pub backtick: Vec<DaemonBacktick>,
    /// Environment variables to set.
    pub setenv: Vec<(OsString, OsString)>,
    /// Environment variables to unset.
    pub unsetenv: Vec<OsString>,
}

impl PtySessionConfig {
    pub fn new(
        socket_path: impl Into<PathBuf>,
        program: impl Into<OsString>,
        args: Vec<OsString>,
    ) -> Self {
        Self {
            socket_path: socket_path.into(),
            program: program.into(),
            args,
            size: PtySize::new(80, 24),
            terminal: OsString::from("screen"),
            log_path: None,
            working_directory: None,
            client_timeout: Duration::from_secs(5),
            output_buffer_limit: 1024 * 1024,
            hardstatus_format: None,
            scrollback_limit: None,
            default_monitor: None,
            default_flow: None,
            default_wrap: None,
            default_silence: None,
            auto_nuke: None,
            startup_windows: Vec::new(),
            ignorecase: None,
            compacthist: None,
            bufferfile: None,
            markkeys: None,
            vbell: None,
            vbell_msg: None,
            bell_msg: None,
            autodetach: None,
            scrollback: None,
            msgwait: None,
            bce: None,
            defutf8: None,
            defencoding: None,
            slowpaste: None,
            sessionname: None,
            maxwin: None,
            crlf: None,
            printcmd: None,
            hardcopy_append: None,
            nonblock: None,
            zmodem: None,
            wall: None,
            backtick: Vec::new(),
            setenv: Vec::new(),
            unsetenv: Vec::new(),
        }
    }

    pub fn with_terminal(mut self, terminal: impl Into<OsString>) -> Self {
        self.terminal = terminal.into();
        self
    }
}

pub fn run_pty_session(config: PtySessionConfig) -> Result<(), DaemonError> {
    ensure_parent_exists(&config.socket_path)?;
    reject_existing_socket_path(&config.socket_path)?;

    signal::install();

    let listener = UnixListener::bind(&config.socket_path).map_err(|source| DaemonError::Bind {
        path: config.socket_path.clone(),
        source,
    })?;
    listener
        .set_nonblocking(true)
        .map_err(DaemonError::Accept)?;
    let _cleanup = SocketCleanup::new(config.socket_path.clone());

    let sty = sty_value(&config.socket_path);

    // Create the initial window
    let mut session = SessionState::new();
    session.hardstatus_format = config.hardstatus_format.clone();
    session.default_monitor = config.default_monitor;
    session.default_wrap = config.default_wrap;
    session.default_silence = config.default_silence;
    session.auto_nuke = config.auto_nuke;
    if let Some(enabled) = config.default_flow {
        session.flow_control = enabled;
    }
    if let Some(v) = config.ignorecase {
        session.ignorecase = v;
    }
    session.maxwin = config.maxwin;
    if let Some(v) = config.autodetach {
        session.autodetach = v;
    }
    session.printcmd = config.printcmd.clone();
    if let Some(v) = config.hardcopy_append {
        session.hardcopy_append = v;
    }
    if let Some(v) = config.zmodem {
        session.zmodem = v;
    }
    session.wall = config.wall.clone();
    session.backtick = config.backtick.clone();
    // setenv/unsetenv are applied by the CLI before daemon start
    let _window0 = session.create_window(
        &config.program,
        &config.args,
        config.size,
        &config.terminal,
        &sty,
        config.working_directory.as_deref(),
        config.scrollback_limit,
    )?;

    // Execute startup windows from .screenrc (screen, title, stuff commands)
    for sw in &config.startup_windows {
        let program = sw.program.clone().unwrap_or_else(|| config.program.clone());
        let _result = session.create_window(
            &program,
            &sw.args,
            config.size,
            &config.terminal,
            &sty,
            sw.working_directory
                .as_ref()
                .map(|p| Path::new(p.as_os_str()))
                .or(config.working_directory.as_deref()),
            config.scrollback_limit,
        )?;
        // Set window title if specified
        if let Some(title) = &sw.title
            && let Some(win) = session.windows.last_mut()
        {
            win.terminal.apply(b"\x1b]2;");
            win.terminal.apply(title);
            win.terminal.apply(b"\x07");
        }
        // Select specific number
        if let Some(number) = sw.number {
            let _ = session.select_window(number);
        }
        // Stuff initial text
        if let Some(stuff) = &sw.stuff {
            let _ = session.write_to_window(session.windows.len() - 1, stuff);
        }
    }

    let mut log_file = open_log_file(config.log_path.as_deref())?;
    let (client_tx, client_rx) = mpsc::channel();
    let mut clients: Vec<AttachedClient> = Vec::new();
    let mut next_client_id = 1_u64;

    loop {
        match signal::poll() {
            Some(DaemonSignal::Shutdown) => {
                // Detach all clients gracefully, then exit
                for mut client in clients.drain(..) {
                    let _ = Message::Detach.write_to(&mut client.stream);
                }
                return Ok(());
            }
            Some(DaemonSignal::DetachClients) => {
                // SIGHUP: detach all connected clients
                for mut client in clients.drain(..) {
                    let _ = Message::Detach.write_to(&mut client.stream);
                }
            }
            None => {}
        }

        // Accept new clients and handle one-shot commands
        if accept_connections(
            &listener,
            &mut clients,
            &mut next_client_id,
            &mut session,
            &config,
            config.client_timeout,
            &client_tx,
        )? {
            return Ok(());
        }

        // Process client events
        while let Ok(event) = client_rx.try_recv() {
            match event {
                ClientEvent::Input(id, bytes) => {
                    // If copy mode is active, redirect keystrokes to copy mode
                    if session.copy_mode_active {
                        if let Some(c) = clients.iter().find(|c| c.id == id) {
                            let selected = c.selected;
                            match bytes.as_slice() {
                                // Escape or Ctrl-c or q: exit copy mode
                                [0x1b] | [0x03] | [b'q'] => {
                                    session.copy_mode_active = false;
                                    session.copy_mode_mark = None;
                                    if let Some(window) = session.windows.get(selected) {
                                        let redraw = window.grid_redraw();
                                        if let Some(c) = clients.iter_mut().find(|c| c.id == id) {
                                            let _ =
                                                Message::PtyOutput(redraw).write_to(&mut c.stream);
                                        }
                                    }
                                }
                                // j or down: move cursor down
                                [b'j'] | [b'B'] => {
                                    session.copy_mode_cursor =
                                        session.copy_mode_cursor.saturating_add(1);
                                    send_copy_cursor(id, &session, &mut clients)?;
                                }
                                // k or up: move cursor up
                                [b'k'] | [b'A'] => {
                                    session.copy_mode_cursor =
                                        session.copy_mode_cursor.saturating_sub(1);
                                    send_copy_cursor(id, &session, &mut clients)?;
                                }
                                // h or left: move column left (not supported, just acknowledge)
                                [b'h'] | [b'D'] => {}
                                // l or right: move column right
                                [b'l'] | [b'C'] => {}
                                // Space: set mark
                                [b' '] => {
                                    if session.copy_mode_mark.is_none() {
                                        session.copy_mode_mark = Some(session.copy_mode_cursor);
                                    } else {
                                        // Second mark: copy region and exit
                                        let mark = session.copy_mode_mark.unwrap_or(0);
                                        let start = mark.min(session.copy_mode_cursor);
                                        let end = mark.max(session.copy_mode_cursor);
                                        if let Some(window) = session.windows.get(selected) {
                                            let lines = window.scrollback_lines();
                                            let mut selected_data = Vec::new();
                                            for i in start..=end {
                                                if let Some(line) = lines.get(i as usize) {
                                                    selected_data.extend_from_slice(line);
                                                    selected_data.push(b'\n');
                                                }
                                            }
                                            session.paste_buffer.push(selected_data);
                                        }
                                        session.copy_mode_mark = None;
                                        session.copy_mode_active = false;
                                        if let Some(window) = session.windows.get(selected) {
                                            let redraw = window.grid_redraw();
                                            if let Some(c) = clients.iter_mut().find(|c| c.id == id)
                                            {
                                                let _ = Message::PtyOutput(redraw)
                                                    .write_to(&mut c.stream);
                                            }
                                        }
                                    }
                                }
                                // a or enter: copy and exit
                                [b'a'] | [b'\r'] => {
                                    let mark =
                                        session.copy_mode_mark.unwrap_or(session.copy_mode_cursor);
                                    let start = mark.min(session.copy_mode_cursor);
                                    let end = mark.max(session.copy_mode_cursor);
                                    if let Some(window) = session.windows.get(selected) {
                                        let lines = window.scrollback_lines();
                                        let mut selected_data = Vec::new();
                                        for i in start..=end {
                                            if let Some(line) = lines.get(i as usize) {
                                                selected_data.extend_from_slice(line);
                                                selected_data.push(b'\n');
                                            }
                                        }
                                        session.paste_buffer.push(selected_data);
                                    }
                                    session.copy_mode_mark = None;
                                    session.copy_mode_active = false;
                                    if let Some(window) = session.windows.get(selected) {
                                        let redraw = window.grid_redraw();
                                        if let Some(c) = clients.iter_mut().find(|c| c.id == id) {
                                            let _ =
                                                Message::PtyOutput(redraw).write_to(&mut c.stream);
                                        }
                                    }
                                }
                                // g or G: go to top/bottom
                                [b'g'] => {
                                    session.copy_mode_cursor = 0;
                                    send_copy_cursor(id, &session, &mut clients)?;
                                }
                                [b'G'] => {
                                    if let Some(window) = session.windows.get(selected) {
                                        let total = window.scrollback_lines().len() as u32;
                                        session.copy_mode_cursor = total.saturating_sub(1);
                                        send_copy_cursor(id, &session, &mut clients)?;
                                    }
                                }
                                // Ctrl-f: page down
                                [0x06] => {
                                    if let Some(window) = session.windows.get(selected) {
                                        let rows = window.terminal.dimensions.rows as u32;
                                        let total = window.scrollback_lines().len() as u32;
                                        session.copy_mode_cursor = (session.copy_mode_cursor
                                            + rows)
                                            .min(total.saturating_sub(1));
                                        send_copy_cursor(id, &session, &mut clients)?;
                                    }
                                }
                                // Ctrl-b: page up
                                [0x02] => {
                                    session.copy_mode_cursor =
                                        session.copy_mode_cursor.saturating_sub(
                                            session
                                                .windows
                                                .get(selected)
                                                .map(|w| w.terminal.dimensions.rows)
                                                .unwrap_or(24)
                                                as u32,
                                        );
                                    send_copy_cursor(id, &session, &mut clients)?;
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(client) = clients.iter().find(|c| c.id == id) {
                        session.write_to_window(client.selected, &bytes)?;
                    }
                }
                ClientEvent::Resize(id, size) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id) {
                        session.resize_window(client.selected, size)?;
                    }
                }
                ClientEvent::Detach(id) => {
                    detach_client(&mut clients, id)?;
                }
                ClientEvent::Shutdown => return Ok(()),
                ClientEvent::SelectWindow(id, number) => {
                    let new_idx = session.window_index(number);
                    if let Some(client) = clients.iter_mut().find(|c| c.id == id) {
                        if let Some(idx) = new_idx {
                            client.last_selected = client.selected;
                            client.selected = idx;
                        }
                        let _ = Message::WindowSelected { number }.write_to(&mut client.stream);
                    }
                }
                ClientEvent::NextWindow(id) => {
                    if let Some(client) = clients.iter_mut().find(|c| c.id == id) {
                        let current = client.selected;
                        if let Some(new_idx) = session.next_window_index(current) {
                            client.last_selected = current;
                            client.selected = new_idx;
                            let number = session.windows[new_idx].number;
                            let _ = Message::WindowSelected { number }.write_to(&mut client.stream);
                        }
                    }
                }
                ClientEvent::PrevWindow(id) => {
                    if let Some(client) = clients.iter_mut().find(|c| c.id == id) {
                        let current = client.selected;
                        if let Some(new_idx) = session.prev_window_index(current) {
                            client.last_selected = current;
                            client.selected = new_idx;
                            let number = session.windows[new_idx].number;
                            let _ = Message::WindowSelected { number }.write_to(&mut client.stream);
                        }
                    }
                }
                ClientEvent::CreateWindow(id, program, args) => {
                    let result = session.create_window(
                        &OsString::from_vec(program),
                        &args
                            .iter()
                            .map(|a| OsString::from_vec(a.clone()))
                            .collect::<Vec<_>>(),
                        config.size,
                        &config.terminal,
                        &sty,
                        config.working_directory.as_deref(),
                        config.scrollback_limit,
                    );
                    match result {
                        Ok(win) => {
                            send_to_client(
                                &mut clients,
                                id,
                                &Message::WindowCreated {
                                    id: win.window_id.0,
                                    number: win.number,
                                },
                            )?;
                        }
                        Err(e) => {
                            let err = format!("window creation failed: {e}").into_bytes();
                            send_to_client(&mut clients, id, &Message::Error(err))?;
                        }
                    }
                }
                ClientEvent::KillWindow(_id, number) => {
                    if let Some(dead) = session.kill_window(number)? {
                        broadcast(
                            &mut clients,
                            &Message::WindowExited {
                                id: dead.window_id.0,
                                number: dead.number,
                            },
                        )?;
                        if session.auto_nuke.unwrap_or(false) {
                            session.remove_dead_windows();
                        }
                        if session.is_empty() {
                            broadcast(&mut clients, &Message::ChildExited(0))?;
                            return Ok(());
                        }
                        // Fix up any client viewing the killed window
                        let killed_idx = session
                            .windows
                            .iter()
                            .position(|w| w.number == number)
                            .unwrap_or(usize::MAX);
                        for client in clients.iter_mut() {
                            if client.selected == killed_idx
                                && let Some(new_idx) = session.next_window_index(killed_idx)
                            {
                                client.selected = new_idx;
                                let new_number = session.windows[new_idx].number;
                                let _ = Message::WindowSelected { number: new_number }
                                    .write_to(&mut client.stream);
                            }
                        }
                    }
                }
                ClientEvent::ListWindows(id) => {
                    send_window_list_to_client(&mut clients, id, &session)?;
                }
                ClientEvent::CopyModeRequest(id) => {
                    session.copy_mode_active = !session.copy_mode_active;
                    session.copy_mode_cursor = 0;
                    session.copy_mode_mark = None;
                    if let Some(c) = clients.iter_mut().find(|c| c.id == id) {
                        if session.copy_mode_active {
                            if let Some(window) = session.windows.get(c.selected) {
                                let lines = window.scrollback_lines();
                                let total = lines.len() as u32;
                                let _ =
                                    Message::CopyModeCursor(0, 0, total).write_to(&mut c.stream);
                                let _ = Message::CopyModeData(lines).write_to(&mut c.stream);
                            }
                        } else {
                            if let Some(window) = session.windows.get(c.selected) {
                                let redraw = window.grid_redraw();
                                let _ = Message::PtyOutput(redraw).write_to(&mut c.stream);
                            }
                        }
                    }
                }
                ClientEvent::PasteRequest(_id, data) => {
                    session.paste_buffer.push(data);
                }
                ClientEvent::RenumberWindow(id, new_number) => {
                    let old_number = session.windows.get(session.selected).map(|w| w.number);
                    if let Some(old) = old_number {
                        // Only renumber if the new number is different
                        if new_number != old {
                            // Check no other window has this number
                            let conflict = session.windows.iter().any(|w| w.number == new_number);
                            if !conflict {
                                if let Some(w) = session.windows.get_mut(session.selected) {
                                    w.number = new_number;
                                }
                                let _ = Message::WindowSelected { number: new_number }.write_to(
                                    &mut clients
                                        .iter_mut()
                                        .find(|c| c.id == id)
                                        .map(|c| &mut c.stream)
                                        .unwrap(),
                                );
                            }
                        }
                    }
                }
                ClientEvent::Redisplay => {
                    // Send a full terminal redraw and hardstatus to every attached client
                    if let Some(window) = session.windows.get(session.selected) {
                        let redraw = window.grid_redraw();
                        if !redraw.is_empty() {
                            broadcast(&mut clients, &Message::PtyOutput(redraw))?;
                        }
                    }
                    if session.hardstatus_format.is_some() {
                        let status = session.format_hardstatus();
                        if !status.is_empty() {
                            broadcast(&mut clients, &Message::HardstatusLine(status))?;
                        }
                    }
                }
                ClientEvent::RemoveWindow(_id, number) => {
                    session.remove_window(number);
                }
                ClientEvent::WipeDeadWindows => {
                    session.remove_dead_windows();
                }
                ClientEvent::Echo(text) => {
                    broadcast(&mut clients, &Message::HardstatusLine(text))?;
                }
                ClientEvent::LogToggle(enable) => {
                    session.logging = enable;
                }
                ClientEvent::LogFile(path) => {
                    session.log_file =
                        Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(path)));
                }
                ClientEvent::OtherWindow(id) => {
                    if let Some(client) = clients.iter_mut().find(|c| c.id == id) {
                        std::mem::swap(&mut client.last_selected, &mut client.selected);
                        let number = session.windows[client.selected].number;
                        let _ = Message::WindowSelected { number }.write_to(&mut client.stream);
                    }
                }
                ClientEvent::MonitorToggle(id, enable) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id)
                        && let Some(window) = session.windows.get_mut(client.selected)
                    {
                        window.monitored = enable;
                    }
                }
                ClientEvent::Silence(id, seconds) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id)
                        && let Some(window) = session.windows.get_mut(client.selected)
                    {
                        window.silence_timeout = seconds;
                    }
                }
                ClientEvent::WrapToggle(id, enable) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id)
                        && let Some(window) = session.windows.get_mut(client.selected)
                    {
                        window.wrap_enabled = enable;
                    }
                }
                ClientEvent::ReadBuf(id) => {
                    let exchange_path = session.exchange_file.clone();
                    let path =
                        exchange_path.unwrap_or_else(|| PathBuf::from("/tmp/screen-exchange"));
                    if let Ok(data) = fs::read(&path) {
                        send_to_client(&mut clients, id, &Message::PasteRequest(data))?;
                    }
                }
                ClientEvent::WriteBuf(id, data) => {
                    let exchange_path = session.exchange_file.clone();
                    let path =
                        exchange_path.unwrap_or_else(|| PathBuf::from("/tmp/screen-exchange"));
                    let _ = fs::write(&path, &data);
                    let _ = id;
                }
                ClientEvent::RemoveBuf(id) => {
                    let exchange_path = session.exchange_file.clone();
                    let path =
                        exchange_path.unwrap_or_else(|| PathBuf::from("/tmp/screen-exchange"));
                    let _ = fs::remove_file(&path);
                    let _ = id;
                }
                ClientEvent::RegisterOp(_id, name, data) => {
                    if data.is_empty() {
                        // Get - we could send back but for now no-op
                    } else {
                        let limit = session.registers.len();
                        if limit < 64 || session.registers.contains_key(&name) {
                            session.registers.insert(name, data);
                        }
                    }
                }
                ClientEvent::FlowToggle(id, enable) => {
                    session.flow_control = enable;
                    let _ = id;
                }
                ClientEvent::SendXoff(id) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id) {
                        let _ = session.write_to_window(client.selected, &[0x13]);
                    }
                }
                ClientEvent::SendXon(id) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id) {
                        let _ = session.write_to_window(client.selected, &[0x11]);
                    }
                }
                ClientEvent::BreakSignal(id, _ms) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id) {
                        // Send null bytes as a simple break approximation
                        let _ = session.write_to_window(client.selected, &[0x00; 4]);
                    }
                }
                ClientEvent::WindowInfo(id) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id) {
                        let window = &session.windows[client.selected];
                        let info = format!(
                            "window {} ({})  alive: {}  scrollback: {}\r\n",
                            window.number,
                            String::from_utf8_lossy(
                                window.terminal.title.as_deref().unwrap_or(b"")
                            ),
                            window.alive,
                            window.terminal.scrollback_len()
                        );
                        send_to_client(&mut clients, id, &Message::WindowInfo(info.into_bytes()))?;
                    }
                }
                ClientEvent::SearchHistory(id, query) => {
                    if let Some(client) = clients.iter().find(|c| c.id == id)
                        && let Some(window) = session.windows.get(client.selected)
                    {
                        let lines = window.scrollback_lines();
                        let query_str = String::from_utf8_lossy(&query).to_lowercase();
                        let mut matches: Vec<u32> = Vec::new();
                        for (i, line) in lines.iter().enumerate() {
                            let text = String::from_utf8_lossy(line).to_lowercase();
                            if text.contains(&query_str) {
                                matches.push(i as u32);
                            }
                        }
                        send_to_client(&mut clients, id, &Message::SearchResult(matches))?;
                    }
                }
                ClientEvent::Command(cmd) => {
                    execute_command_string(&cmd, &mut session, &mut clients)?;
                }
                ClientEvent::Hardcopy(_id, number, path) => {
                    if let Some(window) = session.windows.iter().find(|w| w.number == number) {
                        let lines = window.scrollback_lines();
                        let contents: Vec<u8> = lines
                            .iter()
                            .flat_map(|l| {
                                let mut v = l.clone();
                                v.push(b'\n');
                                v
                            })
                            .collect();
                        if let Err(e) = std::fs::write(
                            std::path::PathBuf::from(String::from_utf8_lossy(&path).into_owned()),
                            &contents,
                        ) {
                            let err = format!("hardcopy failed: {e}").into_bytes();
                            broadcast(&mut clients, &Message::Error(err))?;
                        }
                    }
                }
                ClientEvent::SplitVertical(_id) => {
                    // Create a new region showing the next window
                    if session.windows.len() > 1 {
                        let next_idx = session.next_window_index(session.selected).unwrap_or(0);
                        let total_height = session
                            .windows
                            .get(session.selected)
                            .map(|w| w.terminal.dimensions.rows)
                            .unwrap_or(24);
                        let half = total_height / 2;
                        // Recompute regions: top half shows selected, bottom half shows next
                        session.regions = vec![
                            Region {
                                window_idx: session.selected,
                                top: 0,
                                height: half,
                            },
                            Region {
                                window_idx: next_idx,
                                top: half,
                                height: total_height - half,
                            },
                        ];
                        session.focused_region = 0;
                        // Send redraw for each region
                        broadcast_region_layout(&session, &mut clients)?;
                    }
                }
                ClientEvent::RemoveRegion(id) => {
                    if session.regions.len() > 1 {
                        session.regions.remove(session.focused_region);
                        if session.focused_region >= session.regions.len() {
                            session.focused_region = session.regions.len().saturating_sub(1);
                        }
                        if session.regions.len() == 1 {
                            session.selected = session.regions[0].window_idx;
                            session.regions.clear();
                            send_to_client(
                                &mut clients,
                                id,
                                &Message::WindowSelected {
                                    number: session
                                        .windows
                                        .get(session.selected)
                                        .map(|w| w.number)
                                        .unwrap_or(0),
                                },
                            )?;
                        }
                        broadcast_region_layout(&session, &mut clients)?;
                    }
                }
                ClientEvent::OnlyWindow(_id) => {
                    if let Some(saved) = session.saved_regions.take() {
                        // Restore hidden regions
                        session.regions = saved;
                        if session.focused_region >= session.regions.len() {
                            session.focused_region = 0;
                        }
                    } else if !session.regions.is_empty() {
                        // Save and collapse to just current region
                        session.saved_regions = Some(session.regions.clone());
                        session.regions.clear();
                    }
                    broadcast_region_layout(&session, &mut clients)?;
                }
                ClientEvent::FocusNextRegion(id) => {
                    if !session.regions.is_empty() {
                        session.focused_region =
                            (session.focused_region + 1) % session.regions.len();
                        let new = session.regions[session.focused_region].window_idx;
                        session.selected = new;
                        if let Some(win) = session.windows.get(new) {
                            send_to_client(
                                &mut clients,
                                id,
                                &Message::WindowSelected { number: win.number },
                            )?;
                        }
                        broadcast_region_layout(&session, &mut clients)?;
                    }
                }
                ClientEvent::FocusPrevRegion(id) => {
                    if !session.regions.is_empty() {
                        if session.focused_region == 0 {
                            session.focused_region = session.regions.len() - 1;
                        } else {
                            session.focused_region -= 1;
                        }
                        let new = session.regions[session.focused_region].window_idx;
                        session.selected = new;
                        if let Some(win) = session.windows.get(new) {
                            send_to_client(
                                &mut clients,
                                id,
                                &Message::WindowSelected { number: win.number },
                            )?;
                        }
                        broadcast_region_layout(&session, &mut clients)?;
                    }
                }
                ClientEvent::ResizeRegion(_id, delta) => {
                    if session.regions.len() >= 2 {
                        let total_height: u16 = session.regions.iter().map(|r| r.height).sum();
                        let idx = session.focused_region;
                        let other = if idx == 0 { 1 } else { idx - 1 };
                        let new_h = (session.regions[idx].height as i16 + delta)
                            .clamp(3, total_height as i16 - 3)
                            as u16;
                        let old_h = session.regions[idx].height;
                        session.regions[idx].height = new_h;
                        session.regions[other].height = (session.regions[other].height as i16
                            + (old_h as i16 - new_h as i16))
                            as u16;
                        // Recompute tops
                        let mut top = 0u16;
                        for region in session.regions.iter_mut() {
                            region.top = top;
                            top += region.height;
                        }
                        broadcast_region_layout(&session, &mut clients)?;
                    }
                }
                ClientEvent::CopyModeMove(id, delta) => {
                    if session.copy_mode_active {
                        let new_pos = session.copy_mode_cursor as i64 + delta as i64;
                        session.copy_mode_cursor = new_pos.max(0) as u32;
                        if let Some(c) = clients.iter_mut().find(|c| c.id == id)
                            && let Some(window) = session.windows.get(c.selected)
                        {
                            let total = window.scrollback_lines().len() as u32;
                            if session.copy_mode_cursor >= total {
                                session.copy_mode_cursor = total.saturating_sub(1);
                            }
                            let _ = Message::CopyModeCursor(session.copy_mode_cursor, 0, total)
                                .write_to(&mut c.stream);
                        }
                    }
                }
                ClientEvent::CopyModeMark(id) => {
                    if session.copy_mode_active {
                        if let Some(mark) = session.copy_mode_mark {
                            let start = mark.min(session.copy_mode_cursor);
                            let end = mark.max(session.copy_mode_cursor);
                            if let Some(c) = clients.iter_mut().find(|c| c.id == id)
                                && let Some(window) = session.windows.get(c.selected)
                            {
                                let lines = window.scrollback_lines();
                                let mut selected = Vec::new();
                                for i in start..=end {
                                    if let Some(line) = lines.get(i as usize) {
                                        selected.extend_from_slice(line);
                                        selected.push(b'\n');
                                    }
                                }
                                session.paste_buffer.push(selected);
                            }
                            session.copy_mode_mark = None;
                            session.copy_mode_active = false;
                            if let Some(c) = clients.iter_mut().find(|c| c.id == id)
                                && let Some(window) = session.windows.get(c.selected)
                            {
                                let redraw = window.grid_redraw();
                                let _ = Message::PtyOutput(redraw).write_to(&mut c.stream);
                            }
                        } else {
                            session.copy_mode_mark = Some(session.copy_mode_cursor);
                        }
                    }
                }
                ClientEvent::CopyModeCopy(id) => {
                    if session.copy_mode_active {
                        let mark = session.copy_mode_mark.unwrap_or(session.copy_mode_cursor);
                        let start = mark.min(session.copy_mode_cursor);
                        let end = mark.max(session.copy_mode_cursor);
                        if let Some(c) = clients.iter_mut().find(|c| c.id == id)
                            && let Some(window) = session.windows.get(c.selected)
                        {
                            let lines = window.scrollback_lines();
                            let mut selected = Vec::new();
                            for i in start..=end {
                                if let Some(line) = lines.get(i as usize) {
                                    selected.extend_from_slice(line);
                                    selected.push(b'\n');
                                }
                            }
                            session.paste_buffer.push(selected);
                        }
                    }
                }
                ClientEvent::CopyModePaste(_id, data) => {
                    let _ = session.write_to_window(session.selected, &data);
                }
                ClientEvent::AtWindow(_id, number, data) => {
                    // Send input to a specific window by number
                    if let Some(idx) = session.window_index(number) {
                        let _ = session.write_to_window(idx, &data);
                    }
                }
            }
        }

        // Poll all windows for output
        let mut any_exited = false;
        for (idx, window) in session.windows.iter_mut().enumerate() {
            if !window.is_alive() {
                continue;
            }
            if let Some(pty) = &mut window.pty {
                let output = pty.read_available()?;
                if !output.is_empty() {
                    // Feed output through the terminal engine for scrollback tracking
                    let old_title = window.terminal.title.clone();
                    window.terminal.apply(&output);
                    window.last_activity = SystemTime::now();

                    // Broadcast title change to all clients
                    if window.terminal.title != old_title
                        && let Some(ref title) = window.terminal.title
                    {
                        broadcast(
                            &mut clients,
                            &Message::WindowTitle {
                                number: window.number,
                                title: title.clone(),
                            },
                        )?;
                    }

                    // Activity monitoring: notify clients if window is monitored and not focused
                    if window.monitored {
                        let focused = clients.iter().any(|c| c.selected == idx);
                        if !focused {
                            let msg = format!("Activity in window {}", window.number);
                            session.last_message = msg.clone().into_bytes();
                            for client in clients.iter_mut() {
                                let _ = Message::Activity(msg.clone().into_bytes())
                                    .write_to(&mut client.stream);
                            }
                        }
                    }

                    // Bell detection: notify clients when output contains BEL (0x07)
                    if output.contains(&0x07) {
                        for client in clients.iter_mut() {
                            let _ = Message::Bell(b"bell".to_vec()).write_to(&mut client.stream);
                        }
                    }

                    // Silence monitoring: check for windows with silence timeout
                    if window.silence_timeout > 0 {
                        let elapsed = window
                            .last_activity
                            .elapsed()
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        if elapsed >= u64::from(window.silence_timeout) {
                            let msg = format!("Silence in window {}", window.number);
                            session.last_message = msg.clone().into_bytes();
                            for client in clients.iter_mut() {
                                let _ = Message::HardstatusLine(msg.clone().into_bytes())
                                    .write_to(&mut client.stream);
                            }
                        }
                    }

                    // Log output if logging is enabled
                    if let (Some(log_file), Some(log_path)) =
                        (&mut log_file, config.log_path.as_deref())
                    {
                        log_file
                            .write_all(&output)
                            .map_err(|source| DaemonError::Io {
                                path: log_path.to_owned(),
                                source,
                            })?;
                        log_file.flush().map_err(|source| DaemonError::Io {
                            path: log_path.to_owned(),
                            source,
                        })?;
                    }
                    // Also log to session-level log file if logging is enabled
                    if session.logging
                        && let Some(ref log_path) = session.log_file
                        && let Ok(mut f) =
                            OpenOptions::new().create(true).append(true).open(log_path)
                    {
                        let _ = f.write_all(&output);
                        let _ = f.flush();
                    }
                    window.buffer_output(&output, config.output_buffer_limit);
                    // Send to clients that have this window selected
                    broadcast_to_clients_viewing(&mut clients, idx, &output)?;
                }
            }
        }

        // Check for child exits
        let mut exited_windows = Vec::new();
        for (idx, window) in session.windows.iter_mut().enumerate() {
            if let Some(pty) = &mut window.pty
                && let Some(status) = pty.wait_timeout(Duration::from_millis(0))?
            {
                window.mark_exited(status.code().unwrap_or(-1));
                exited_windows.push((idx, window.number, window.id));
            }
        }

        for (idx, number, window_id) in exited_windows {
            broadcast(
                &mut clients,
                &Message::WindowExited {
                    id: window_id.0,
                    number,
                },
            )?;
            // For each client viewing this window, auto-switch to another
            for client in clients.iter_mut() {
                if client.selected == idx {
                    if let Some(new_idx) = session.next_window_index(idx) {
                        client.selected = new_idx;
                        let new_number = session.windows[new_idx].number;
                        let _ = Message::WindowSelected { number: new_number }
                            .write_to(&mut client.stream);
                    } else {
                        // No windows left for this client
                        let _ = Message::ChildExited(0).write_to(&mut client.stream);
                    }
                }
            }
            // Also update session.selected if it pointed to the dead window
            if session.selected == idx {
                let _ = session.select_next_alive();
            }
            any_exited = true;
        }

        if any_exited {
            // Auto-nuke: remove dead windows immediately if configured.
            if session.auto_nuke.unwrap_or(false) {
                session.remove_dead_windows();
            }
            // Dead windows are kept as zombies — visible in the list but not
            // auto-switched to. Explicit removal is done via -X wipe or remove.
            if session.windows.iter().all(|w| !w.alive) {
                return Ok(());
            }
        }

        // Generate and broadcast hardstatus if configured
        if session.hardstatus_format.is_some() {
            let status = session.format_hardstatus();
            if !status.is_empty() && !clients.is_empty() {
                broadcast(&mut clients, &Message::HardstatusLine(status))?;
            }
        }

        thread::sleep(Duration::from_millis(10));
    }
}

// ─── Session & Window management ───────────────────────────────────────────

#[derive(Debug)]
struct SessionState {
    windows: Vec<ManagedWindow>,
    selected: usize,
    next_id: u64,
    next_number: u32,
    paste_buffer: Vec<Vec<u8>>,
    hardstatus_format: Option<Vec<u8>>,
    logging: bool,
    log_file: Option<std::path::PathBuf>,
    /// Named registers for copy mode.
    registers: std::collections::HashMap<u8, Vec<u8>>,
    /// Exchange file path for readbuf/writebuf.
    exchange_file: Option<PathBuf>,
    /// Flow control state.
    flow_control: bool,
    /// Last message displayed via Echo/Activity/etc.
    last_message: Vec<u8>,
    /// Config defaults for new windows.
    default_monitor: Option<bool>,
    default_wrap: Option<bool>,
    default_silence: Option<u16>,
    auto_nuke: Option<bool>,
    /// Region-based split layout.
    regions: Vec<Region>,
    /// Index into regions: which region has focus.
    focused_region: usize,
    /// Saved regions for only/unsplit restore.
    saved_regions: Option<Vec<Region>>,
    /// Copy mode state.
    copy_mode_active: bool,
    copy_mode_cursor: u32,
    copy_mode_mark: Option<u32>,
    /// Search case sensitivity.
    ignorecase: bool,
    /// Max windows.
    maxwin: Option<u32>,
    /// Auto-detach on hangup.
    autodetach: bool,
    /// Hardcopy print command.
    printcmd: Option<OsString>,
    /// Hardcopy append mode.
    hardcopy_append: bool,
    /// Zmodem catch.
    zmodem: bool,
    /// Wall message.
    wall: Option<Vec<u8>>,
    /// Backtick commands to run.
    backtick: Vec<DaemonBacktick>,
}

#[derive(Debug, Clone)]
struct Region {
    /// Index into self.windows for the window displayed in this region.
    window_idx: usize,
    /// Top row of this region in the composite display.
    top: u16,
    /// Height of this region in rows.
    height: u16,
}

#[derive(Debug)]
struct ManagedWindow {
    id: screen_core::WindowId,
    number: u32,
    pty: Option<PtyProcess>,
    output_buffer: Vec<u8>,
    alive: bool,
    exit_code: Option<i32>,
    terminal: TerminalState,
    /// Whether activity monitoring is enabled for this window.
    monitored: bool,
    /// Silence timeout in seconds (0 = disabled).
    silence_timeout: u16,
    /// Last time output was received from the pty.
    last_activity: SystemTime,
    /// Whether line wrapping is enabled.
    wrap_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct WindowCreated {
    pub window_id: screen_core::WindowId,
    pub number: u32,
}

#[derive(Debug, Clone)]
pub struct WindowDead {
    pub window_id: screen_core::WindowId,
    pub number: u32,
}

impl SessionState {
    fn new() -> Self {
        Self {
            windows: Vec::new(),
            selected: 0,
            next_id: 1,
            next_number: 0,
            paste_buffer: Vec::new(),
            hardstatus_format: None,
            logging: false,
            log_file: None,
            registers: std::collections::HashMap::new(),
            exchange_file: None,
            flow_control: false,
            last_message: Vec::new(),
            default_monitor: None,
            default_wrap: None,
            default_silence: None,
            auto_nuke: None,
            regions: Vec::new(),
            focused_region: 0,
            saved_regions: None,
            copy_mode_active: false,
            copy_mode_cursor: 0,
            copy_mode_mark: None,
            ignorecase: true,
            maxwin: None,
            autodetach: false,
            printcmd: None,
            hardcopy_append: false,
            zmodem: false,
            wall: None,
            backtick: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create_window(
        &mut self,
        program: &OsStr,
        args: &[OsString],
        size: PtySize,
        term: &OsStr,
        sty: &OsStr,
        working_directory: Option<&Path>,
        scrollback_limit: Option<u32>,
    ) -> Result<WindowCreated, DaemonError> {
        // Enforce maxwin limit
        if let Some(max) = self.maxwin
            && self.windows.len() >= max as usize
        {
            return Err(DaemonError::MaxWindowsExceeded {
                max,
                current: self.windows.len(),
            });
        }
        let id = screen_core::WindowId(self.next_id);
        self.next_id += 1;
        let number = self.next_number;
        self.next_number += 1;

        let mut cmd = PtyCommand::new(program, size);
        cmd.args(args.iter());
        if let Some(wd) = working_directory {
            cmd.current_dir(wd);
        }
        cmd.env("STY", sty);
        cmd.env("WINDOW", number.to_string().as_str());
        cmd.env("TERM", term);

        let pty = cmd.spawn()?;
        let mut terminal = TerminalState::new(Dimensions::new(size.columns, size.rows));
        if let Some(limit) = scrollback_limit {
            terminal.set_scrollback_limit(limit);
        }

        let window = ManagedWindow {
            id,
            number,
            pty: Some(pty),
            output_buffer: Vec::new(),
            alive: true,
            exit_code: None,
            terminal,
            monitored: self.default_monitor.unwrap_or(false),
            silence_timeout: self.default_silence.unwrap_or(0),
            last_activity: SystemTime::now(),
            wrap_enabled: self.default_wrap.unwrap_or(true),
        };

        self.windows.push(window);
        let idx = self.windows.len() - 1;
        if self.windows.len() == 1 {
            self.selected = idx;
        }

        Ok(WindowCreated {
            window_id: id,
            number,
        })
    }

    fn write_to_selected(&mut self, bytes: &[u8]) -> Result<(), DaemonError> {
        self.write_to_window(self.selected, bytes)
    }

    fn write_to_window(&mut self, idx: usize, bytes: &[u8]) -> Result<(), DaemonError> {
        if let Some(window) = self.windows.get_mut(idx)
            && let Some(pty) = &mut window.pty
        {
            pty.write_all(bytes).map_err(DaemonError::Pty)?;
        }
        Ok(())
    }

    fn resize_window(&mut self, idx: usize, size: PtySize) -> Result<(), DaemonError> {
        if let Some(window) = self.windows.get_mut(idx) {
            if let Some(pty) = &window.pty {
                pty.resize(size)?;
            }
            window
                .terminal
                .resize(Dimensions::new(size.columns, size.rows));
        }
        Ok(())
    }

    fn window_index(&self, number: u32) -> Option<usize> {
        self.windows
            .iter()
            .position(|w| w.number == number && w.alive)
    }

    fn select_window(&mut self, number: u32) -> Result<(), String> {
        if let Some(idx) = self.window_index(number) {
            self.selected = idx;
            Ok(())
        } else {
            Err(format!("no window with number {number}"))
        }
    }

    fn select_next_alive(&mut self) -> Option<u32> {
        self.next_window_index(self.selected).map(|idx| {
            self.selected = idx;
            self.windows[idx].number
        })
    }

    fn next_window_index(&self, current: usize) -> Option<usize> {
        let len = self.windows.len();
        if len == 0 {
            return None;
        }
        for offset in 1..=len {
            let idx = (current + offset) % len;
            if self.windows[idx].alive {
                return Some(idx);
            }
        }
        None
    }

    fn prev_window_index(&self, current: usize) -> Option<usize> {
        let len = self.windows.len();
        if len == 0 {
            return None;
        }
        for offset in 1..=len {
            let idx = (current + len - offset) % len;
            if self.windows[idx].alive {
                return Some(idx);
            }
        }
        None
    }

    fn next_window(&mut self) -> Option<&ManagedWindow> {
        self.next_window_index(self.selected).map(|idx| {
            self.selected = idx;
            &self.windows[idx]
        })
    }

    fn prev_window(&mut self) -> Option<&ManagedWindow> {
        self.prev_window_index(self.selected).map(|idx| {
            self.selected = idx;
            &self.windows[idx]
        })
    }

    fn kill_window(&mut self, number: u32) -> Result<Option<WindowDead>, DaemonError> {
        if let Some(idx) = self.windows.iter().position(|w| w.number == number) {
            // Take the PTY and kill it without blocking
            if let Some(pty) = self.windows[idx].pty.take() {
                // Kill the process group - PtyProcess::Drop will wait up to 1s
                // To avoid blocking, we spawn a thread to handle cleanup
                std::thread::spawn(move || {
                    drop(pty);
                });
            }
            self.windows[idx].alive = false;
            self.windows[idx].exit_code = Some(-1);
            let dead = WindowDead {
                window_id: self.windows[idx].id,
                number: self.windows[idx].number,
            };
            return Ok(Some(dead));
        }
        Ok(None)
    }

    #[allow(dead_code)]
    fn remove_dead_windows(&mut self) {
        let mut new_windows = Vec::new();
        let mut offset = 0_usize;
        for (idx, window) in self.windows.drain(..).enumerate() {
            if window.alive {
                new_windows.push(window);
            } else if idx < self.selected {
                offset += 1;
            }
        }
        self.selected = self
            .selected
            .saturating_sub(offset)
            .min(new_windows.len().saturating_sub(1));
        self.windows = new_windows;
    }

    fn remove_window(&mut self, number: u32) {
        if let Some(idx) = self.windows.iter().position(|w| w.number == number) {
            // Only allow removing dead/zombie windows
            if !self.windows[idx].alive {
                self.windows.remove(idx);
                if self.selected >= self.windows.len() && !self.windows.is_empty() {
                    self.selected = self.windows.len() - 1;
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    #[allow(dead_code)]
    fn format_hardstatus(&self) -> Vec<u8> {
        let Some(format) = &self.hardstatus_format else {
            return Vec::new();
        };
        let active_number = self
            .windows
            .get(self.selected)
            .map(|w| w.number)
            .unwrap_or(0);
        let active_title = self
            .windows
            .get(self.selected)
            .and_then(|w| w.terminal.title.clone())
            .unwrap_or_default();
        let winfos: Vec<screen_core::hardstatus::WindowInfo> = self
            .windows
            .iter()
            .filter(|w| w.alive)
            .map(|w| screen_core::hardstatus::WindowInfo {
                number: w.number,
                flags: if w.number == active_number { 1 } else { 0 },
                title: w.terminal.title.clone().unwrap_or_default(),
            })
            .collect();
        screen_core::hardstatus::expand_hardstatus(
            format,
            active_number,
            &active_title,
            &winfos,
            SystemTime::now(),
        )
    }
}

impl ManagedWindow {
    fn is_alive(&self) -> bool {
        self.alive
    }

    fn mark_exited(&mut self, code: i32) {
        self.alive = false;
        self.exit_code = Some(code);
    }

    fn buffer_output(&mut self, bytes: &[u8], limit: usize) {
        self.output_buffer.extend_from_slice(bytes);
        if self.output_buffer.len() > limit {
            let excess = self.output_buffer.len() - limit;
            self.output_buffer.drain(..excess);
        }
    }

    /// Build a full-screen redraw from the terminal grid.
    /// Returns escape sequences to clear the screen, draw each line,
    /// and reposition the cursor.
    fn grid_redraw(&self) -> Vec<u8> {
        let mut dump = Vec::with_capacity(4096);
        let rows = self.terminal.dimensions.rows;
        // Clear screen, home cursor
        dump.extend_from_slice(b"\x1b[H\x1b[J");
        for row in 0..rows {
            if let Some(line) = self.terminal.line_bytes(row) {
                dump.extend_from_slice(&line);
            }
            if row + 1 < rows {
                dump.extend_from_slice(b"\r\n");
            }
        }
        // Restore cursor position
        let cursor_pos = format!(
            "\x1b[{};{}H",
            self.terminal.cursor.row + 1,
            self.terminal.cursor.column + 1
        );
        dump.extend_from_slice(cursor_pos.as_bytes());
        dump
    }

    /// Return scrollback lines derived from the terminal engine.
    /// Combines scrollback buffer lines and visible grid lines.
    fn scrollback_lines(&self) -> Vec<Vec<u8>> {
        let mut lines: Vec<Vec<u8>> = Vec::new();
        // Scrollback buffer (oldest first)
        for i in 0..self.terminal.scrollback_len() {
            let idx = self.terminal.scrollback_len() - 1 - i;
            if let Some(line) = self.terminal.scrollback_line(idx) {
                lines.push(line);
            }
        }
        // Visible grid rows
        for row in 0..self.terminal.dimensions.rows {
            if let Some(line) = self.terminal.line_bytes(row) {
                lines.push(line);
            }
        }
        lines
    }
}

#[allow(clippy::ptr_arg)]
fn send_window_list_to_client(
    clients: &mut Vec<AttachedClient>,
    client_id: u64,
    session: &SessionState,
) -> Result<(), DaemonError> {
    // Find the client's selected window number
    let client_selected_num = clients
        .iter()
        .find(|c| c.id == client_id)
        .and_then(|c| session.windows.get(c.selected))
        .map(|w| w.number);

    let list: Vec<WindowInfoMsg> = session
        .windows
        .iter()
        .map(|w| {
            let selected = Some(w.number) == client_selected_num;
            let dead = !w.alive;
            let flags: u8 = if selected && dead {
                3
            } else if selected {
                1
            } else if dead {
                2
            } else {
                0
            };
            WindowInfoMsg {
                number: w.number,
                flags,
                title: w.terminal.title.clone().unwrap_or_default(),
            }
        })
        .collect();

    for client in clients.iter_mut() {
        if client.id == client_id {
            Message::WindowList(list.clone()).write_to(&mut client.stream)?;
        }
    }
    Ok(())
}

fn broadcast_region_layout(
    session: &SessionState,
    clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    if !session.regions.is_empty() {
        let layout: Vec<(u32, u16, u16, bool)> = session
            .regions
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                session
                    .windows
                    .get(r.window_idx)
                    .map(|w| (w.number, r.top, r.height, i == session.focused_region))
            })
            .collect();
        if !layout.is_empty() {
            broadcast(clients, &Message::RegionLayout(layout))?;
        }
    }
    Ok(())
}

fn send_copy_cursor(
    id: u64,
    session: &SessionState,
    #[allow(clippy::ptr_arg)] clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    if let Some(c) = clients.iter_mut().find(|c| c.id == id)
        && let Some(window) = session.windows.get(c.selected)
    {
        let total = window.scrollback_lines().len() as u32;
        let cursor = session.copy_mode_cursor.min(total.saturating_sub(1));
        let _ = Message::CopyModeCursor(cursor, 0, total).write_to(&mut c.stream);
    }
    Ok(())
}

#[allow(clippy::ptr_arg)]
fn send_to_client(
    clients: &mut Vec<AttachedClient>,
    client_id: u64,
    message: &Message,
) -> Result<(), DaemonError> {
    for client in clients.iter_mut() {
        if client.id == client_id {
            message.write_to(&mut client.stream)?;
        }
    }
    Ok(())
}

// ─── Client handling ───────────────────────────────────────────────────────

#[derive(Debug)]
struct AttachedClient {
    id: u64,
    stream: UnixStream,
    /// Index into session.windows this client is viewing
    selected: usize,
    /// Previously-selected window index for "other" command
    last_selected: usize,
}

#[derive(Debug)]
enum ClientEvent {
    Input(u64, Vec<u8>),
    Resize(u64, PtySize),
    Detach(u64),
    Shutdown,
    CreateWindow(u64, Vec<u8>, Vec<Vec<u8>>),
    SelectWindow(u64, u32),
    KillWindow(u64, u32),
    NextWindow(u64),
    PrevWindow(u64),
    ListWindows(u64),
    CopyModeRequest(u64),
    PasteRequest(u64, Vec<u8>),
    RenumberWindow(u64, u32),
    Redisplay,
    RemoveWindow(u64, u32),
    WipeDeadWindows,
    Echo(Vec<u8>),
    LogToggle(bool),
    LogFile(Vec<u8>),
    OtherWindow(u64),
    MonitorToggle(u64, bool),
    Silence(u64, u16),
    WrapToggle(u64, bool),
    ReadBuf(u64),
    WriteBuf(u64, Vec<u8>),
    RemoveBuf(u64),
    RegisterOp(u64, u8, Vec<u8>),
    FlowToggle(u64, bool),
    SendXoff(u64),
    SendXon(u64),
    BreakSignal(u64, u16),
    WindowInfo(u64),
    SearchHistory(u64, Vec<u8>),
    /// Execute an arbitrary screen command string.
    Command(Vec<u8>),
    /// Write terminal contents to a file.
    Hardcopy(u64, u32, Vec<u8>),
    /// Region split/control.
    SplitVertical(u64),
    RemoveRegion(u64),
    OnlyWindow(u64),
    FocusNextRegion(u64),
    FocusPrevRegion(u64),
    ResizeRegion(u64, i16),
    /// Copy mode operations.
    CopyModeMove(u64, i32),
    CopyModeMark(u64),
    CopyModeCopy(u64),
    CopyModePaste(u64, Vec<u8>),
    /// Send input to a specific window by number.
    AtWindow(u64, u32, Vec<u8>),
}

fn handle_client(stream: &mut UnixStream) -> Result<ClientOutcome, DaemonError> {
    match Message::read_from(stream) {
        Ok(Message::Hello) => Message::HelloAck.write_to(stream)?,
        Ok(message) => {
            write_protocol_error(stream, format!("expected hello, received {message:?}"))?;
            return Ok(ClientOutcome::Continue);
        }
        Err(error) => {
            write_protocol_error(stream, format!("malformed hello: {error}"))?;
            return Ok(ClientOutcome::Continue);
        }
    }

    match Message::read_from(stream) {
        Ok(Message::Shutdown) => {
            Message::ShutdownAck.write_to(stream)?;
            Ok(ClientOutcome::Shutdown)
        }
        Ok(message) => {
            write_protocol_error(stream, format!("expected shutdown, received {message:?}"))?;
            Ok(ClientOutcome::Continue)
        }
        Err(error) => {
            write_protocol_error(stream, format!("malformed command: {error}"))?;
            Ok(ClientOutcome::Continue)
        }
    }
}

fn write_protocol_error(stream: &mut UnixStream, message: String) -> Result<(), DaemonError> {
    let _ = Message::Error(message.into_bytes()).write_to(stream);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientOutcome {
    Continue,
    Shutdown,
}

// ─── Event loop helpers ────────────────────────────────────────────────────

fn accept_connections(
    listener: &UnixListener,
    clients: &mut Vec<AttachedClient>,
    next_client_id: &mut u64,
    session: &mut SessionState,
    config: &PtySessionConfig,
    client_timeout: Duration,
    client_tx: &mpsc::Sender<ClientEvent>,
) -> Result<bool, DaemonError> {
    loop {
        match listener.accept() {
            Ok((mut stream, _address)) => {
                stream
                    .set_nonblocking(false)
                    .map_err(DaemonError::ConfigureClient)?;
                configure_client_timeouts(&stream, client_timeout)?;

                // Hello handshake
                match Message::read_from(&mut stream) {
                    Ok(Message::Hello) => {
                        Message::HelloAck.write_to(&mut stream)?;
                    }
                    Ok(message) => {
                        write_protocol_error(
                            &mut stream,
                            format!("expected hello, received {message:?}"),
                        )?;
                        continue;
                    }
                    Err(error) => {
                        write_protocol_error(&mut stream, format!("malformed hello: {error}"))?;
                        continue;
                    }
                }

                // Process the actual command
                match Message::read_from(&mut stream) {
                    Ok(Message::Attach) => {
                        // Full attach - add to clients list
                        // Send a grid redraw so the client sees current terminal state
                        if let Some(window) = session.windows.get(session.selected) {
                            let redraw = window.grid_redraw();
                            if !redraw.is_empty() {
                                Message::PtyOutput(redraw).write_to(&mut stream)?;
                            }
                        }
                        clear_client_read_timeout(&stream)?;
                        let id = *next_client_id;
                        *next_client_id += 1;
                        spawn_client_reader(
                            id,
                            stream.try_clone().map_err(DaemonError::ConfigureClient)?,
                            client_tx,
                        );
                        clients.push(AttachedClient {
                            id,
                            stream,
                            selected: session.selected,
                            last_selected: session.selected,
                        });
                    }
                    Ok(Message::Detach) => {
                        detach_all_clients(clients)?;
                    }
                    Ok(Message::PtyInput(bytes)) => {
                        session.write_to_selected(&bytes)?;
                    }
                    Ok(Message::Shutdown) => {
                        Message::ShutdownAck.write_to(&mut stream)?;
                        return Ok(true);
                    }
                    Ok(Message::CreateWindow { program, args }) => {
                        let result = session.create_window(
                            &OsString::from_vec(program),
                            &args
                                .iter()
                                .map(|a| OsString::from_vec(a.clone()))
                                .collect::<Vec<_>>(),
                            config.size,
                            &config.terminal,
                            &sty_value(&config.socket_path),
                            config.working_directory.as_deref(),
                            config.scrollback_limit,
                        );
                        match result {
                            Ok(win) => {
                                Message::WindowCreated {
                                    id: win.window_id.0,
                                    number: win.number,
                                }
                                .write_to(&mut stream)?;
                            }
                            Err(e) => {
                                let err = format!("window creation failed: {e}").into_bytes();
                                Message::Error(err).write_to(&mut stream)?;
                            }
                        }
                    }
                    Ok(Message::SelectWindow { number }) => {
                        if session.select_window(number).is_ok() {
                            Message::WindowSelected { number }.write_to(&mut stream)?;
                        } else {
                            Message::Error(b"no such window".to_vec()).write_to(&mut stream)?;
                        }
                    }
                    Ok(Message::KillWindow { number }) => {
                        if let Some(dead) = session.kill_window(number)? {
                            broadcast(
                                clients,
                                &Message::WindowExited {
                                    id: dead.window_id.0,
                                    number: dead.number,
                                },
                            )?;
                            if session.is_empty() {
                                broadcast(clients, &Message::ChildExited(0))?;
                                return Ok(true);
                            }
                            Message::WindowExited {
                                id: dead.window_id.0,
                                number: dead.number,
                            }
                            .write_to(&mut stream)?;
                        } else {
                            Message::Error(b"no such window".to_vec()).write_to(&mut stream)?;
                        }
                    }
                    Ok(Message::NextWindow) => {
                        if let Some(win) = session.next_window() {
                            Message::WindowSelected { number: win.number }.write_to(&mut stream)?;
                        }
                    }
                    Ok(Message::PrevWindow) => {
                        if let Some(win) = session.prev_window() {
                            Message::WindowSelected { number: win.number }.write_to(&mut stream)?;
                        }
                    }
                    Ok(Message::CopyModeRequest) => {
                        // One-shot: return scrollback for selected window
                        if let Some(window) = session.windows.get(session.selected) {
                            let lines = window.scrollback_lines();
                            Message::CopyModeData(lines).write_to(&mut stream)?;
                        }
                    }
                    Ok(Message::PasteRequest(data)) => {
                        session.paste_buffer.push(data);
                    }
                    Ok(Message::RenumberWindow { number }) => {
                        if let Some(selected) = session.windows.get(session.selected) {
                            let old = selected.number;
                            if number != old {
                                let conflict = session.windows.iter().any(|w| w.number == number);
                                if !conflict {
                                    if let Some(w) = session.windows.get_mut(session.selected) {
                                        w.number = number;
                                    }
                                    Message::WindowSelected { number }.write_to(&mut stream)?;
                                }
                            }
                        }
                    }
                    Ok(Message::Redisplay) => {
                        // Send a full terminal redraw to every attached client
                        if let Some(window) = session.windows.get(session.selected) {
                            let redraw = window.grid_redraw();
                            if !redraw.is_empty() {
                                for client in clients.iter_mut() {
                                    let _ = Message::PtyOutput(redraw.clone())
                                        .write_to(&mut client.stream);
                                }
                            }
                        }
                        // Also send hardstatus if configured
                        if session.hardstatus_format.is_some() {
                            let status = session.format_hardstatus();
                            if !status.is_empty() {
                                for client in clients.iter_mut() {
                                    let _ = Message::HardstatusLine(status.clone())
                                        .write_to(&mut client.stream);
                                }
                            }
                        }
                    }
                    Ok(Message::RemoveWindow { number }) => {
                        session.remove_window(number);
                    }
                    Ok(Message::WipeDeadWindows) => {
                        session.remove_dead_windows();
                    }
                    Ok(Message::Echo(text)) => {
                        // Display echo on all attached clients via hardstatus
                        for client in clients.iter_mut() {
                            let _ =
                                Message::HardstatusLine(text.clone()).write_to(&mut client.stream);
                        }
                    }
                    Ok(Message::LogToggle { enable }) => {
                        session.logging = enable;
                    }
                    Ok(Message::LogFile(path)) => {
                        session.log_file =
                            Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(path)));
                    }
                    Ok(Message::MonitorToggle { enable }) => {
                        if let Some(window) = session.windows.get_mut(session.selected) {
                            window.monitored = enable;
                        }
                    }
                    Ok(Message::Silence { seconds }) => {
                        if let Some(window) = session.windows.get_mut(session.selected) {
                            window.silence_timeout = seconds;
                        }
                    }
                    Ok(Message::WrapToggle { enable }) => {
                        if let Some(window) = session.windows.get_mut(session.selected) {
                            window.wrap_enabled = enable;
                        }
                    }
                    Ok(Message::ReadBuf) => {
                        let exchange_path = session.exchange_file.clone();
                        let path =
                            exchange_path.unwrap_or_else(|| PathBuf::from("/tmp/screen-exchange"));
                        if let Ok(data) = fs::read(&path) {
                            // Send paste data back to client via PasteRequest
                            for client in clients.iter_mut() {
                                let _ = Message::PasteRequest(data.clone())
                                    .write_to(&mut client.stream);
                            }
                        }
                    }
                    Ok(Message::WriteBuf(data)) => {
                        let exchange_path = session.exchange_file.clone();
                        let path =
                            exchange_path.unwrap_or_else(|| PathBuf::from("/tmp/screen-exchange"));
                        let _ = fs::write(&path, &data);
                    }
                    Ok(Message::RemoveBuf) => {
                        let exchange_path = session.exchange_file.clone();
                        let path =
                            exchange_path.unwrap_or_else(|| PathBuf::from("/tmp/screen-exchange"));
                        let _ = fs::remove_file(&path);
                    }
                    Ok(Message::Register { name, data }) => {
                        if data.is_empty() {
                            // Get register - send to client
                            let content = session.registers.get(&name).cloned();
                            if let Some(_c) = content {
                                // Send back via PasteRequest or similar
                            }
                        } else {
                            // Set register
                            let limit = session.registers.len();
                            if limit < 64 || session.registers.contains_key(&name) {
                                session.registers.insert(name, data);
                            }
                        }
                    }
                    Ok(Message::FlowToggle { enable }) => {
                        session.flow_control = enable;
                    }
                    Ok(Message::Xoff) => {
                        let _ = session.write_to_selected(&[0x13]);
                    }
                    Ok(Message::Xon) => {
                        let _ = session.write_to_selected(&[0x11]);
                    }
                    Ok(Message::BreakSignal { ms: _ }) => {
                        if let Some(window) = session.windows.get(session.selected)
                            && let Some(_pty) = &window.pty
                        {
                            // Send break by using tcsendbreak if available
                            // For now, send a null byte as a simple break approximation
                        }
                    }
                    Ok(Message::SearchHistory(query)) => {
                        // Search scrollback and respond with matching line indices
                        if let Some(window) = session.windows.get(session.selected) {
                            let lines = window.scrollback_lines();
                            let query_str = String::from_utf8_lossy(&query).to_lowercase();
                            let mut matches: Vec<u32> = Vec::new();
                            for (i, line) in lines.iter().enumerate() {
                                let text = String::from_utf8_lossy(line).to_lowercase();
                                if text.contains(&query_str) {
                                    matches.push(i as u32);
                                }
                            }
                            for client in clients.iter_mut() {
                                let _ = Message::SearchResult(matches.clone())
                                    .write_to(&mut client.stream);
                            }
                        }
                    }
                    Ok(Message::Command(cmd)) => {
                        let mut empty_clients = Vec::new();
                        let _ = execute_command_string(&cmd, session, &mut empty_clients);
                    }
                    Ok(Message::Hardcopy(number, path)) => {
                        if let Some(window) = session.windows.iter().find(|w| w.number == number) {
                            let lines = window.scrollback_lines();
                            let contents: Vec<u8> = lines
                                .iter()
                                .flat_map(|l| {
                                    let mut v = l.clone();
                                    v.push(b'\n');
                                    v
                                })
                                .collect();
                            let file_path = std::path::PathBuf::from(
                                String::from_utf8_lossy(&path).into_owned(),
                            );
                            let _ = std::fs::write(&file_path, &contents);
                        }
                    }
                    Ok(Message::AtWindow(number, data)) => {
                        if let Some(idx) = session.window_index(number) {
                            let _ = session.write_to_window(idx, &data);
                        }
                    }
                    Ok(Message::WindowList(_)) => {
                        let list: Vec<WindowInfoMsg> = session
                            .windows
                            .iter()
                            .map(|w| WindowInfoMsg {
                                number: w.number,
                                flags: if w.number
                                    == session
                                        .windows
                                        .get(session.selected)
                                        .map(|s| s.number)
                                        .unwrap_or(0)
                                {
                                    1
                                } else if !w.alive {
                                    2
                                } else {
                                    0
                                },
                                title: w.terminal.title.clone().unwrap_or_default(),
                            })
                            .collect();
                        Message::WindowList(list).write_to(&mut stream)?;
                    }
                    Ok(message) => {
                        write_protocol_error(
                            &mut stream,
                            format!("unexpected command: {message:?}"),
                        )?;
                    }
                    Err(error) => {
                        write_protocol_error(&mut stream, format!("malformed command: {error}"))?;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(DaemonError::Accept(error)),
        }
    }
}
fn spawn_client_reader(id: u64, mut stream: UnixStream, client_tx: &mpsc::Sender<ClientEvent>) {
    let client_tx = client_tx.clone();
    thread::spawn(move || {
        loop {
            match Message::read_from(&mut stream) {
                Ok(Message::PtyInput(bytes)) => {
                    if client_tx.send(ClientEvent::Input(id, bytes)).is_err() {
                        break;
                    }
                }
                Ok(Message::Resize { columns, rows }) => {
                    if client_tx
                        .send(ClientEvent::Resize(id, PtySize::new(columns, rows)))
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Detach) => {
                    let _ = client_tx.send(ClientEvent::Detach(id));
                    break;
                }
                Ok(Message::Shutdown) => {
                    let _ = client_tx.send(ClientEvent::Shutdown);
                    break;
                }
                Ok(Message::CreateWindow { program, args }) => {
                    let _ = client_tx.send(ClientEvent::CreateWindow(id, program, args));
                }
                Ok(Message::SelectWindow { number }) => {
                    let _ = client_tx.send(ClientEvent::SelectWindow(id, number));
                }
                Ok(Message::KillWindow { number }) => {
                    let _ = client_tx.send(ClientEvent::KillWindow(id, number));
                }
                Ok(Message::NextWindow) => {
                    let _ = client_tx.send(ClientEvent::NextWindow(id));
                }
                Ok(Message::PrevWindow) => {
                    let _ = client_tx.send(ClientEvent::PrevWindow(id));
                }
                Ok(Message::WindowList(_)) => {
                    let _ = client_tx.send(ClientEvent::ListWindows(id));
                }
                Ok(Message::CopyModeRequest) => {
                    let _ = client_tx.send(ClientEvent::CopyModeRequest(id));
                }
                Ok(Message::PasteRequest(data)) => {
                    let _ = client_tx.send(ClientEvent::PasteRequest(id, data));
                }
                Ok(Message::RenumberWindow { number }) => {
                    let _ = client_tx.send(ClientEvent::RenumberWindow(id, number));
                }
                Ok(Message::Redisplay) => {
                    let _ = client_tx.send(ClientEvent::Redisplay);
                }
                Ok(Message::RemoveWindow { number }) => {
                    let _ = client_tx.send(ClientEvent::RemoveWindow(id, number));
                }
                Ok(Message::WipeDeadWindows) => {
                    let _ = client_tx.send(ClientEvent::WipeDeadWindows);
                }
                Ok(Message::Echo(text)) => {
                    let _ = client_tx.send(ClientEvent::Echo(text));
                }
                Ok(Message::LogToggle { enable }) => {
                    let _ = client_tx.send(ClientEvent::LogToggle(enable));
                }
                Ok(Message::LogFile(path)) => {
                    let _ = client_tx.send(ClientEvent::LogFile(path));
                }
                Ok(Message::OtherWindow) => {
                    let _ = client_tx.send(ClientEvent::OtherWindow(id));
                }
                Ok(Message::MonitorToggle { enable }) => {
                    let _ = client_tx.send(ClientEvent::MonitorToggle(id, enable));
                }
                Ok(Message::Silence { seconds }) => {
                    let _ = client_tx.send(ClientEvent::Silence(id, seconds));
                }
                Ok(Message::WrapToggle { enable }) => {
                    let _ = client_tx.send(ClientEvent::WrapToggle(id, enable));
                }
                Ok(Message::ReadBuf) => {
                    let _ = client_tx.send(ClientEvent::ReadBuf(id));
                }
                Ok(Message::WriteBuf(data)) => {
                    let _ = client_tx.send(ClientEvent::WriteBuf(id, data));
                }
                Ok(Message::RemoveBuf) => {
                    let _ = client_tx.send(ClientEvent::RemoveBuf(id));
                }
                Ok(Message::Register { name, data }) => {
                    let _ = client_tx.send(ClientEvent::RegisterOp(id, name, data));
                }
                Ok(Message::FlowToggle { enable }) => {
                    let _ = client_tx.send(ClientEvent::FlowToggle(id, enable));
                }
                Ok(Message::Xoff) => {
                    let _ = client_tx.send(ClientEvent::SendXoff(id));
                }
                Ok(Message::Xon) => {
                    let _ = client_tx.send(ClientEvent::SendXon(id));
                }
                Ok(Message::BreakSignal { ms }) => {
                    let _ = client_tx.send(ClientEvent::BreakSignal(id, ms));
                }
                Ok(Message::WindowInfo(_info)) => {
                    // Forward window info from daemon to client
                    let _ = client_tx.send(ClientEvent::WindowInfo(id));
                }
                Ok(Message::SearchHistory(query)) => {
                    let _ = client_tx.send(ClientEvent::SearchHistory(id, query));
                }
                Ok(Message::Command(cmd)) => {
                    let _ = client_tx.send(ClientEvent::Command(cmd));
                }
                Ok(Message::Hardcopy(number, path)) => {
                    let _ = client_tx.send(ClientEvent::Hardcopy(id, number, path));
                }
                Ok(Message::SplitVertical) => {
                    let _ = client_tx.send(ClientEvent::SplitVertical(id));
                }
                Ok(Message::RemoveRegion) => {
                    let _ = client_tx.send(ClientEvent::RemoveRegion(id));
                }
                Ok(Message::OnlyWindow) => {
                    let _ = client_tx.send(ClientEvent::OnlyWindow(id));
                }
                Ok(Message::FocusNext) => {
                    let _ = client_tx.send(ClientEvent::FocusNextRegion(id));
                }
                Ok(Message::FocusPrev) => {
                    let _ = client_tx.send(ClientEvent::FocusPrevRegion(id));
                }
                Ok(Message::ResizeRegion(delta)) => {
                    let _ = client_tx.send(ClientEvent::ResizeRegion(id, delta));
                }
                Ok(Message::CopyModeMove(delta)) => {
                    let _ = client_tx.send(ClientEvent::CopyModeMove(id, delta));
                }
                Ok(Message::CopyModeMark) => {
                    let _ = client_tx.send(ClientEvent::CopyModeMark(id));
                }
                Ok(Message::CopyModeCopy) => {
                    let _ = client_tx.send(ClientEvent::CopyModeCopy(id));
                }
                Ok(Message::CopyModePaste(data)) => {
                    let _ = client_tx.send(ClientEvent::CopyModePaste(id, data));
                }
                Ok(Message::AtWindow(number, data)) => {
                    let _ = client_tx.send(ClientEvent::AtWindow(id, number, data));
                }
                Ok(_message) => {}
                Err(_error) => {
                    let _ = client_tx.send(ClientEvent::Detach(id));
                    break;
                }
            }
        }
    });
}

#[allow(clippy::ptr_arg)]
fn broadcast(clients: &mut Vec<AttachedClient>, message: &Message) -> Result<(), DaemonError> {
    let mut i = 0;
    while i < clients.len() {
        if message.write_to(&mut clients[i].stream).is_err() {
            clients.remove(i);
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Broadcast PTY output only to clients viewing the given window index.
fn broadcast_to_clients_viewing(
    clients: &mut Vec<AttachedClient>,
    window_idx: usize,
    output: &[u8],
) -> Result<(), DaemonError> {
    let msg = Message::PtyOutput(output.to_vec());
    let mut i = 0;
    while i < clients.len() {
        if clients[i].selected == window_idx && msg.write_to(&mut clients[i].stream).is_err() {
            clients.remove(i);
            continue;
        }
        i += 1;
    }
    Ok(())
}

fn detach_client(clients: &mut Vec<AttachedClient>, id: u64) -> Result<(), DaemonError> {
    if let Some(pos) = clients.iter().position(|c| c.id == id) {
        let mut client = clients.remove(pos);
        let _ = Message::Detach.write_to(&mut client.stream);
    }
    Ok(())
}

#[allow(clippy::ptr_arg)]
fn detach_all_clients(clients: &mut Vec<AttachedClient>) -> Result<(), DaemonError> {
    for mut client in clients.drain(..) {
        let _ = Message::Detach.write_to(&mut client.stream);
    }
    Ok(())
}

fn configure_client_timeouts(stream: &UnixStream, timeout: Duration) -> Result<(), DaemonError> {
    set_socket_timeout(stream.set_read_timeout(Some(timeout)))?;
    set_socket_timeout(stream.set_write_timeout(Some(timeout)))?;
    Ok(())
}

fn clear_client_read_timeout(stream: &UnixStream) -> Result<(), DaemonError> {
    set_socket_timeout(stream.set_read_timeout(None))
}

fn set_socket_timeout(result: io::Result<()>) -> Result<(), DaemonError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => Ok(()),
        Err(error) => Err(DaemonError::ConfigureClient(error)),
    }
}

// ─── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum DaemonError {
    Io { path: PathBuf, source: io::Error },
    SocketPathExists { path: PathBuf },
    Bind { path: PathBuf, source: io::Error },
    Accept(io::Error),
    ConfigureClient(io::Error),
    Protocol(ProtocolError),
    Pty(PtyError),
    MaxWindowsExceeded { max: u32, current: usize },
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::SocketPathExists { path } => {
                write!(formatter, "socket path already exists: {}", path.display())
            }
            Self::Bind { path, source } => {
                write!(formatter, "failed to bind {}: {source}", path.display())
            }
            Self::Accept(error) => write!(formatter, "failed to accept daemon client: {error}"),
            Self::ConfigureClient(error) => {
                write!(
                    formatter,
                    "failed to configure daemon client socket: {error}"
                )
            }
            Self::Protocol(error) => write!(formatter, "{error}"),
            Self::Pty(error) => write!(formatter, "{error}"),
            Self::MaxWindowsExceeded { max, current } => {
                write!(formatter, "max windows ({max}) exceeded ({current})")
            }
        }
    }
}

impl Error for DaemonError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::Bind { source, .. } => Some(source),
            Self::Accept(error) | Self::ConfigureClient(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Pty(error) => Some(error),
            Self::MaxWindowsExceeded { .. } | Self::SocketPathExists { .. } => None,
        }
    }
}

impl From<ProtocolError> for DaemonError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl From<PtyError> for DaemonError {
    fn from(error: PtyError) -> Self {
        Self::Pty(error)
    }
}

// ─── Socket cleanup ────────────────────────────────────────────────────────

struct SocketCleanup {
    path: PathBuf,
}

impl SocketCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn ensure_parent_exists(path: &Path) -> Result<(), DaemonError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|source| DaemonError::Io {
        path: parent.to_owned(),
        source,
    })
}

fn reject_existing_socket_path(path: &Path) -> Result<(), DaemonError> {
    match fs::symlink_metadata(path) {
        Ok(_metadata) => Err(DaemonError::SocketPathExists {
            path: path.to_owned(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(DaemonError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn sty_value(socket_path: &Path) -> OsString {
    socket_path
        .file_name()
        .unwrap_or_else(|| OsStr::new("screen-rs"))
        .to_owned()
}

fn open_log_file(path: Option<&Path>) -> Result<Option<File>, DaemonError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| DaemonError::Io {
            path: path.to_owned(),
            source,
        })?;
    Ok(Some(file))
}

enum DaemonSignal {
    DetachClients,
    Shutdown,
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod signal {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SIGHUP: AtomicBool = AtomicBool::new(false);
    static SHUTDOWN: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle_sighup(_: libc::c_int) {
        SIGHUP.store(true, Ordering::SeqCst);
    }
    extern "C" fn handle_shutdown(_: libc::c_int) {
        SHUTDOWN.store(true, Ordering::SeqCst);
    }

    pub fn install() {
        unsafe {
            libc::signal(
                libc::SIGHUP,
                handle_sighup as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGTERM,
                handle_shutdown as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGINT,
                handle_shutdown as *const () as libc::sighandler_t,
            );
        }
    }

    pub fn poll() -> Option<super::DaemonSignal> {
        if SHUTDOWN.swap(false, Ordering::SeqCst) {
            Some(super::DaemonSignal::Shutdown)
        } else if SIGHUP.swap(false, Ordering::SeqCst) {
            Some(super::DaemonSignal::DetachClients)
        } else {
            None
        }
    }
}

#[cfg(not(unix))]
mod signal {
    pub fn install() {}
    pub fn poll() -> Option<super::DaemonSignal> {
        None
    }
}

/// Parse and execute a simple screen command string (for -X colon).
/// Supports common commands like: select N, monitor on/off, wrap on/off, etc.
fn execute_command_string(
    cmd: &[u8],
    session: &mut SessionState,
    clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    let text = String::from_utf8_lossy(cmd);
    let mut parts = text.split_whitespace();
    let Some(command) = parts.next() else {
        return Ok(());
    };
    match command {
        "select" => {
            if let Some(num_str) = parts.next()
                && let Ok(num) = num_str.parse::<u32>()
            {
                let _ = session.select_window(num);
            }
        }
        "monitor" => {
            let enable = parts.next().is_none_or(|a| a != "off");
            if let Some(window) = session.windows.get_mut(session.selected) {
                window.monitored = enable;
            }
        }
        "silence" => {
            if let Some(sec_str) = parts.next()
                && let Ok(sec) = sec_str.parse::<u16>()
                && let Some(window) = session.windows.get_mut(session.selected)
            {
                window.silence_timeout = sec;
            }
        }
        "wrap" => {
            let enable = parts.next().is_none_or(|a| a != "off");
            if let Some(window) = session.windows.get_mut(session.selected) {
                window.wrap_enabled = enable;
            }
        }
        "log" => {
            let enable = parts.next().is_none_or(|a| a != "off");
            session.logging = enable;
        }
        "flow" => {
            let enable = parts.next().is_none_or(|a| a != "off");
            session.flow_control = enable;
        }
        "redisplay" => {
            if let Some(window) = session.windows.get(session.selected) {
                let redraw = window.grid_redraw();
                for client in clients.iter_mut() {
                    let _ = Message::PtyOutput(redraw.clone()).write_to(&mut client.stream);
                }
            }
        }
        "echo" => {
            let msg = parts.clone().collect::<Vec<_>>().join(" ");
            for client in clients.iter_mut() {
                let _ =
                    Message::HardstatusLine(msg.clone().into_bytes()).write_to(&mut client.stream);
            }
        }
        "kill" => {
            if let Some(window) = session.windows.get(session.selected) {
                let number = window.number;
                if let Some(dead) = session.kill_window(number)? {
                    broadcast(
                        clients,
                        &Message::WindowExited {
                            id: dead.window_id.0,
                            number: dead.number,
                        },
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::process;

    #[test]
    fn sty_value_uses_daemon_pid_and_session_name() {
        let expected = format!("{}.envcase", process::id());
        let path = PathBuf::from("/tmp/screen-rs").join(&expected);
        let sty = sty_value(&path);
        assert_eq!(sty, OsString::from(expected));
    }

    #[test]
    fn sty_value_preserves_non_utf8_session_name_bytes() {
        let mut path = PathBuf::from("/tmp/screen-rs");
        let mut name = process::id().to_string().into_bytes();
        name.extend_from_slice(b".n\xffme");
        path.push(OsString::from_vec(name.clone()));

        let sty = sty_value(&path);
        assert_eq!(sty.as_bytes(), name.as_slice());
    }
}
