#![forbid(unsafe_code)]

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
    let _window0 = session.create_window(
        &config.program,
        &config.args,
        config.size,
        &config.terminal,
        &sty,
        config.working_directory.as_deref(),
        config.scrollback_limit,
    )?;
    let mut log_file = open_log_file(config.log_path.as_deref())?;
    let (client_tx, client_rx) = mpsc::channel();
    let mut clients: Vec<AttachedClient> = Vec::new();
    let mut next_client_id = 1_u64;

    loop {
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
                    if let Some(client) = clients.iter().find(|c| c.id == id) {
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
                            client.selected = idx;
                        }
                        let _ = Message::WindowSelected { number }.write_to(&mut client.stream);
                    }
                }
                ClientEvent::NextWindow(id) => {
                    if let Some(client) = clients.iter_mut().find(|c| c.id == id) {
                        let current = client.selected;
                        if let Some(new_idx) = session.next_window_index(current) {
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
                    if let Some(client) = clients.iter().find(|c| c.id == id)
                        && let Some(window) = session.windows.get(client.selected)
                    {
                        let lines = window.scrollback_lines();
                        if let Some(c) = clients.iter_mut().find(|c| c.id == id) {
                            Message::CopyModeData(lines).write_to(&mut c.stream)?;
                        }
                    }
                }
                ClientEvent::PasteRequest(_id, data) => {
                    session.paste_buffer.push(data);
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
                    window.terminal.apply(&output);

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
            session.remove_dead_windows();
            if session.is_empty() {
                return Ok(());
            }
        }

        // Generate and broadcast hardstatus if configured
        if session.hardstatus_format.is_some() {
            let status = session.format_hardstatus();
            if !status.is_empty() && !clients.is_empty() {
                broadcast(&mut clients, &Message::PtyOutput(status))?;
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

    fn is_empty(&self) -> bool {
        self.windows.iter().all(|w| !w.alive)
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
        .filter(|w| w.alive)
        .map(|w| {
            let flags: u8 = if Some(w.number) == client_selected_num {
                1 // flag for selected
            } else {
                0
            };
            WindowInfoMsg {
                number: w.number,
                flags,
                title: Vec::new(),
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
                    Ok(Message::WindowList(_)) => {
                        let list: Vec<WindowInfoMsg> = session
                            .windows
                            .iter()
                            .filter(|w| w.alive)
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
                                } else {
                                    0
                                },
                                title: Vec::new(),
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
            Self::SocketPathExists { .. } => None,
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
