#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegisterId(pub u8);

// ---------------------------------------------------------------------------
// Names and states
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionName(pub OsString);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Starting,
    Attached,
    Detached,
    Exiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowState {
    Running,
    Exited,
    Killed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// Not in copy mode
    Off,
    /// Navigating scrollback to select text
    Navigating,
    /// Selection in progress (start point set)
    Selecting { start_row: u32, start_col: u16 },
}

// ---------------------------------------------------------------------------
// Scrollback configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbackConfig {
    pub max_lines: u32,
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self { max_lines: 1000 }
    }
}

// ---------------------------------------------------------------------------
// Session model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub name: SessionName,
    pub created_at: SystemTime,
    pub socket_path: PathBuf,
    pub state: SessionState,
    pub paste_buffer: Vec<Vec<u8>>,
    pub registers: [Option<Vec<u8>>; 26],
}

impl Session {
    pub fn new(id: SessionId, name: SessionName, socket_path: PathBuf) -> Self {
        const NONE: Option<Vec<u8>> = None;
        Self {
            id,
            name,
            created_at: SystemTime::now(),
            socket_path,
            state: SessionState::Starting,
            paste_buffer: Vec::new(),
            registers: [NONE; 26],
        }
    }

    /// Add text to the paste buffer (from copy mode or exchange file).
    pub fn add_to_paste_buffer(&mut self, text: Vec<u8>) {
        self.paste_buffer.push(text);
    }

    /// Get the most recent paste buffer entry.
    pub fn latest_paste(&self) -> Option<&[u8]> {
        self.paste_buffer.last().map(|v| v.as_slice())
    }

    /// Store text in a named register (a-z).
    pub fn set_register(&mut self, id: RegisterId, text: Vec<u8>) {
        let idx = id.0.min(25) as usize;
        self.registers[idx] = Some(text);
    }

    /// Get text from a named register.
    pub fn get_register(&self, id: RegisterId) -> Option<&[u8]> {
        let idx = id.0.min(25) as usize;
        self.registers[idx].as_deref()
    }
}

// ---------------------------------------------------------------------------
// Window model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub number: u32,
    pub title: Vec<u8>,
    pub state: WindowState,
    pub copy_mode: CopyMode,
    pub scrollback_config: ScrollbackConfig,
}

impl WindowInfo {
    pub fn new(id: WindowId, number: u32) -> Self {
        Self {
            id,
            number,
            title: Vec::new(),
            state: WindowState::Running,
            copy_mode: CopyMode::Off,
            scrollback_config: ScrollbackConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Display model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayDimensions {
    pub columns: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub selected_window: WindowId,
    pub dimensions: DisplayDimensions,
    pub terminal_name: Vec<u8>,
}
