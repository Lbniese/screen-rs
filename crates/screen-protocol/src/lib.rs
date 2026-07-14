#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_PAYLOAD_LEN: u32 = 64 * 1024;
const MAGIC: [u8; 4] = *b"SRSP";
const HEADER_LEN: usize = 11;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub protocol_version: u16,
}

impl Hello {
    pub fn current() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Hello,
    HelloAck,
    Shutdown,
    ShutdownAck,
    Attach,
    Detach,
    PtyInput(Vec<u8>),
    PtyOutput(Vec<u8>),
    ChildExited(i32),
    Resize {
        columns: u16,
        rows: u16,
    },
    Error(Vec<u8>),
    // Multi-window messages
    CreateWindow {
        program: Vec<u8>,
        args: Vec<Vec<u8>>,
    },
    WindowCreated {
        id: u64,
        number: u32,
    },
    SelectWindow {
        number: u32,
    },
    WindowSelected {
        number: u32,
    },
    KillWindow {
        number: u32,
    },
    WindowExited {
        id: u64,
        number: u32,
    },
    WindowList(Vec<WindowInfoMsg>),
    NextWindow,
    PrevWindow,
    WindowTitle {
        number: u32,
        title: Vec<u8>,
    },
    CopyModeRequest,
    CopyModeData(Vec<Vec<u8>>),
    PasteRequest(Vec<u8>),
    HardstatusLine(Vec<u8>),
    /// Caption line (always visible, distinct from hardstatus).
    CaptionLine(Vec<u8>),
    RenumberWindow {
        number: u32,
    },
    /// Request the daemon to redraw all attached clients' displays.
    Redisplay,
    /// Remove a specific dead (zombie) window from the session.
    RemoveWindow {
        number: u32,
    },
    /// Remove all dead windows from the session.
    WipeDeadWindows,
    /// Suspend session (C-a z / SIGTSTP).
    Suspend,
    /// Display a short message on all attached clients.
    Echo(Vec<u8>),
    /// Enable or disable logging for the current window.
    LogToggle {
        enable: bool,
    },
    /// Set the log file path for logging.
    LogFile(Vec<u8>),
    /// Toggle to the previously-visited window for this client.
    OtherWindow,
    /// Toggle activity monitoring for the current window.
    MonitorToggle {
        enable: bool,
    },
    /// Notification from daemon that a monitored window had activity.
    Activity(Vec<u8>),
    /// Set silence monitoring timeout for the current window (0 = off).
    Silence {
        seconds: u16,
    },
    /// Bell notification from daemon.
    Bell(Vec<u8>),
    /// Toggle line wrapping for the current window.
    WrapToggle {
        enable: bool,
    },
    /// Read the exchange file into the paste buffer.
    ReadBuf,
    /// Write the paste buffer to the exchange file.
    WriteBuf(Vec<u8>),
    /// Remove the exchange file.
    RemoveBuf,
    /// Named register operation (name 0 = get, non-zero data = set, empty data = get).
    Register {
        name: u8,
        data: Vec<u8>,
    },
    /// Flow control: enable or disable XON/XOFF handling.
    FlowToggle {
        enable: bool,
    },
    /// Send XOFF (Ctrl-S) to the current window.
    Xoff,
    /// Send XON (Ctrl-Q) to the current window.
    Xon,
    /// Send a break signal to the current window.
    BreakSignal {
        ms: u16,
    },
    /// Window info response from daemon.
    WindowInfo(Vec<u8>),
    /// Search scrollback history for a pattern.
    SearchHistory(Vec<u8>),
    /// Search history results.
    SearchResult(Vec<u32>),
    /// Execute an arbitrary screen command string (for -X colon).
    Command(Vec<u8>),
    /// Send input to a specific window by number (-X at).
    AtWindow(u32, Vec<u8>),
    /// Write current terminal contents to a file (hardcopy).
    Hardcopy(u32, Vec<u8>),
    SplitVertical,
    SplitHorizontal,
    RemoveRegion,
    OnlyWindow,
    FocusNext,
    FocusPrev,
    ResizeRegion(i16),
    RegionLayout(Vec<(u32, u16, u16, u16, u16, bool)>),
    /// Copy mode: move cursor up/down in scrollback.
    CopyModeMove(i32),
    /// Copy mode: set mark at current position.
    CopyModeMark,
    /// Copy mode: copy marked region to buffer and exit.
    CopyModeCopy,
    /// Copy mode: insert register contents at current window cursor.
    CopyModePaste(Vec<u8>),
    /// Copy mode cursor position broadcast: (line_index, column, total_lines).
    CopyModeCursor(u32, u16, u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfoMsg {
    pub number: u32,
    pub flags: u8,
    pub title: Vec<u8>,
    pub group: Option<Vec<u8>>,
}

impl Message {
    pub fn write_to(&self, writer: &mut impl Write) -> Result<(), ProtocolError> {
        let child_status_payload;
        let resize_payload;
        let window_created_payload;
        let window_selected_payload;
        let window_exited_payload;
        let create_window_payload;
        let select_window_payload;
        let kill_window_payload;
        let window_title_payload;
        let copy_mode_payload;
        let renumber_window_payload;
        let remove_window_payload;
        let log_toggle_payload;
        let monitor_toggle_payload;
        let silence_payload;
        let wrap_toggle_payload;
        let register_payload;
        let flow_toggle_payload;
        let break_payload;
        let search_result_payload;
        let hardcopy_payload;
        let resize_region_payload;
        let region_layout_payload;
        let at_window_payload;
        let copy_move_payload;
        let copy_cursor_payload;
        let (kind, payload): (MessageKind, &[u8]) = match self {
            Self::Hello => (MessageKind::Hello, &[][..]),
            Self::HelloAck => (MessageKind::HelloAck, &[][..]),
            Self::Shutdown => (MessageKind::Shutdown, &[][..]),
            Self::ShutdownAck => (MessageKind::ShutdownAck, &[][..]),
            Self::Attach => (MessageKind::Attach, &[][..]),
            Self::Detach => (MessageKind::Detach, &[][..]),
            Self::NextWindow => (MessageKind::NextWindow, &[][..]),
            Self::PrevWindow => (MessageKind::PrevWindow, &[][..]),
            Self::PtyInput(payload) => (MessageKind::PtyInput, checked_payload(payload)?),
            Self::PtyOutput(payload) => (MessageKind::PtyOutput, checked_payload(payload)?),
            Self::ChildExited(status) => {
                child_status_payload = status.to_be_bytes();
                (MessageKind::ChildExited, &child_status_payload[..])
            }
            Self::Resize { columns, rows } => {
                resize_payload = [
                    (columns >> 8) as u8,
                    *columns as u8,
                    (rows >> 8) as u8,
                    *rows as u8,
                ];
                (MessageKind::Resize, &resize_payload[..])
            }
            Self::Error(payload) => (MessageKind::Error, checked_payload(payload)?),
            Self::WindowCreated { id, number } => {
                window_created_payload = [
                    (id >> 56) as u8,
                    (id >> 48) as u8,
                    (id >> 40) as u8,
                    (id >> 32) as u8,
                    (id >> 24) as u8,
                    (id >> 16) as u8,
                    (id >> 8) as u8,
                    *id as u8,
                    (number >> 24) as u8,
                    (number >> 16) as u8,
                    (number >> 8) as u8,
                    *number as u8,
                ];
                (MessageKind::WindowCreated, &window_created_payload[..])
            }
            Self::WindowSelected { number } => {
                window_selected_payload = number.to_be_bytes();
                (MessageKind::WindowSelected, &window_selected_payload[..])
            }
            Self::WindowExited { id, number } => {
                window_exited_payload = [
                    (id >> 56) as u8,
                    (id >> 48) as u8,
                    (id >> 40) as u8,
                    (id >> 32) as u8,
                    (id >> 24) as u8,
                    (id >> 16) as u8,
                    (id >> 8) as u8,
                    *id as u8,
                    (number >> 24) as u8,
                    (number >> 16) as u8,
                    (number >> 8) as u8,
                    *number as u8,
                ];
                (MessageKind::WindowExited, &window_exited_payload[..])
            }
            Self::CreateWindow { program, args } => {
                create_window_payload = encode_create_window(program, args);
                (
                    MessageKind::CreateWindow,
                    checked_payload(&create_window_payload)?,
                )
            }
            Self::SelectWindow { number } => {
                select_window_payload = number.to_be_bytes();
                (MessageKind::SelectWindow, &select_window_payload[..])
            }
            Self::KillWindow { number } => {
                kill_window_payload = number.to_be_bytes();
                (MessageKind::KillWindow, &kill_window_payload[..])
            }
            Self::WindowList(list) => {
                create_window_payload = encode_window_list(list);
                (
                    MessageKind::WindowList,
                    checked_payload(&create_window_payload)?,
                )
            }
            Self::WindowTitle { number, title } => {
                window_title_payload = encode_window_title(*number, title);
                (
                    MessageKind::WindowTitle,
                    checked_payload(&window_title_payload)?,
                )
            }
            Self::CopyModeRequest => (MessageKind::CopyModeRequest, &[][..]),
            Self::CopyModeData(lines) => {
                copy_mode_payload = encode_copy_mode_data(lines);
                (
                    MessageKind::CopyModeData,
                    checked_payload(&copy_mode_payload)?,
                )
            }
            Self::PasteRequest(data) => (MessageKind::PasteRequest, checked_payload(data)?),
            Self::HardstatusLine(data) => (MessageKind::HardstatusLine, checked_payload(data)?),
            Self::CaptionLine(data) => (MessageKind::CaptionLine, checked_payload(data)?),
            Self::RenumberWindow { number } => {
                renumber_window_payload = number.to_be_bytes();
                (MessageKind::RenumberWindow, &renumber_window_payload[..])
            }
            Self::Redisplay => (MessageKind::Redisplay, &[]),
            Self::RemoveWindow { number } => {
                remove_window_payload = number.to_be_bytes();
                (MessageKind::RemoveWindow, &remove_window_payload[..])
            }
            Self::WipeDeadWindows => (MessageKind::WipeDeadWindows, &[]),
            Self::Suspend => (MessageKind::Suspend, &[]),
            Self::Echo(payload) => (MessageKind::Echo, checked_payload(payload)?),
            Self::LogToggle { enable } => {
                log_toggle_payload = [*enable as u8];
                (MessageKind::LogToggle, &log_toggle_payload[..])
            }
            Self::LogFile(payload) => (MessageKind::LogFile, checked_payload(payload)?),
            Self::OtherWindow => (MessageKind::OtherWindow, &[]),
            Self::MonitorToggle { enable } => {
                monitor_toggle_payload = [*enable as u8];
                (MessageKind::MonitorToggle, &monitor_toggle_payload[..])
            }
            Self::Activity(payload) => (MessageKind::Activity, checked_payload(payload)?),
            Self::Silence { seconds } => {
                silence_payload = seconds.to_be_bytes();
                (MessageKind::Silence, &silence_payload[..])
            }
            Self::Bell(payload) => (MessageKind::Bell, checked_payload(payload)?),
            Self::WrapToggle { enable } => {
                wrap_toggle_payload = [*enable as u8];
                (MessageKind::WrapToggle, &wrap_toggle_payload[..])
            }
            Self::ReadBuf => (MessageKind::ReadBuf, &[]),
            Self::WriteBuf(payload) => (MessageKind::WriteBuf, checked_payload(payload)?),
            Self::RemoveBuf => (MessageKind::RemoveBuf, &[]),
            Self::Register { name, data } => {
                register_payload = encode_register(*name, data);
                (MessageKind::Register, checked_payload(&register_payload)?)
            }
            Self::FlowToggle { enable } => {
                flow_toggle_payload = [*enable as u8];
                (MessageKind::FlowToggle, &flow_toggle_payload[..])
            }
            Self::Xoff => (MessageKind::Xoff, &[]),
            Self::Xon => (MessageKind::Xon, &[]),
            Self::BreakSignal { ms } => {
                break_payload = ms.to_be_bytes();
                (MessageKind::BreakSignal, &break_payload[..])
            }
            Self::WindowInfo(payload) => (MessageKind::WindowInfo, checked_payload(payload)?),
            Self::SearchHistory(payload) => (MessageKind::SearchHistory, checked_payload(payload)?),
            Self::SearchResult(lines) => {
                search_result_payload = encode_search_result(lines);
                (
                    MessageKind::SearchResult,
                    checked_payload(&search_result_payload)?,
                )
            }
            Self::Command(payload) => (MessageKind::Command, checked_payload(payload)?),
            Self::Hardcopy(num, payload) => {
                let len = 4 + payload.len();
                hardcopy_payload = {
                    let mut buf = Vec::with_capacity(len);
                    buf.extend_from_slice(&num.to_be_bytes());
                    buf.extend_from_slice(payload);
                    buf
                };
                (MessageKind::Hardcopy, checked_payload(&hardcopy_payload)?)
            }
            Self::AtWindow(num, payload) => {
                let len = 4 + payload.len();
                at_window_payload = {
                    let mut buf = Vec::with_capacity(len);
                    buf.extend_from_slice(&num.to_be_bytes());
                    buf.extend_from_slice(payload);
                    buf
                };
                (MessageKind::AtWindow, checked_payload(&at_window_payload)?)
            }
            Self::SplitVertical => (MessageKind::SplitVertical, &[][..]),
            Self::SplitHorizontal => (MessageKind::SplitHorizontal, &[][..]),
            Self::RemoveRegion => (MessageKind::RemoveRegion, &[][..]),
            Self::OnlyWindow => (MessageKind::OnlyWindow, &[][..]),
            Self::FocusNext => (MessageKind::FocusNext, &[][..]),
            Self::FocusPrev => (MessageKind::FocusPrev, &[][..]),
            Self::ResizeRegion(delta) => {
                resize_region_payload = delta.to_be_bytes();
                (
                    MessageKind::ResizeRegion,
                    checked_payload(&resize_region_payload)?,
                )
            }
            Self::RegionLayout(regions) => {
                region_layout_payload = encode_region_layout(regions);
                (
                    MessageKind::RegionLayout,
                    checked_payload(&region_layout_payload)?,
                )
            }
            Self::CopyModeMove(delta) => {
                copy_move_payload = delta.to_be_bytes();
                (
                    MessageKind::CopyModeMove,
                    checked_payload(&copy_move_payload)?,
                )
            }
            Self::CopyModeMark => (MessageKind::CopyModeMark, &[][..]),
            Self::CopyModeCopy => (MessageKind::CopyModeCopy, &[][..]),
            Self::CopyModePaste(data) => (MessageKind::CopyModePaste, checked_payload(data)?),
            Self::CopyModeCursor(line, col, total) => {
                let mut buf = Vec::with_capacity(10);
                buf.extend_from_slice(&line.to_be_bytes());
                buf.extend_from_slice(&col.to_be_bytes());
                buf.extend_from_slice(&total.to_be_bytes());
                copy_cursor_payload = buf;
                (
                    MessageKind::CopyModeCursor,
                    checked_payload(&copy_cursor_payload)?,
                )
            }
        };

        writer.write_all(&MAGIC).map_err(ProtocolError::Io)?;
        writer
            .write_all(&PROTOCOL_VERSION.to_be_bytes())
            .map_err(ProtocolError::Io)?;
        writer.write_all(&[kind as u8]).map_err(ProtocolError::Io)?;
        writer
            .write_all(&(payload.len() as u32).to_be_bytes())
            .map_err(ProtocolError::Io)?;
        writer.write_all(payload).map_err(ProtocolError::Io)?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read) -> Result<Self, ProtocolError> {
        let mut header = [0_u8; HEADER_LEN];
        reader.read_exact(&mut header).map_err(ProtocolError::Io)?;

        if header[0..4] != MAGIC {
            return Err(ProtocolError::BadMagic);
        }

        let version = u16::from_be_bytes([header[4], header[5]]);
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }

        let kind = MessageKind::try_from(header[6])?;
        let payload_len = u32::from_be_bytes([header[7], header[8], header[9], header[10]]);
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::PayloadTooLarge(payload_len as usize));
        }

        let mut payload = vec![0_u8; payload_len as usize];
        reader.read_exact(&mut payload).map_err(ProtocolError::Io)?;

        match kind {
            MessageKind::Hello if payload.is_empty() => Ok(Self::Hello),
            MessageKind::HelloAck if payload.is_empty() => Ok(Self::HelloAck),
            MessageKind::Shutdown if payload.is_empty() => Ok(Self::Shutdown),
            MessageKind::ShutdownAck if payload.is_empty() => Ok(Self::ShutdownAck),
            MessageKind::Attach if payload.is_empty() => Ok(Self::Attach),
            MessageKind::Detach if payload.is_empty() => Ok(Self::Detach),
            MessageKind::NextWindow if payload.is_empty() => Ok(Self::NextWindow),
            MessageKind::PrevWindow if payload.is_empty() => Ok(Self::PrevWindow),
            MessageKind::PtyInput => Ok(Self::PtyInput(payload)),
            MessageKind::PtyOutput => Ok(Self::PtyOutput(payload)),
            MessageKind::ChildExited if payload.len() == 4 => {
                let status = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::ChildExited(status))
            }
            MessageKind::Resize if payload.len() == 4 => {
                let columns = u16::from_be_bytes([payload[0], payload[1]]);
                let rows = u16::from_be_bytes([payload[2], payload[3]]);
                Ok(Self::Resize { columns, rows })
            }
            MessageKind::Error => Ok(Self::Error(payload)),
            MessageKind::WindowCreated if payload.len() == 12 => {
                let id = u64::from_be_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);
                let number = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
                Ok(Self::WindowCreated { id, number })
            }
            MessageKind::WindowSelected if payload.len() == 4 => {
                let number = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::WindowSelected { number })
            }
            MessageKind::WindowExited if payload.len() == 12 => {
                let id = u64::from_be_bytes([
                    payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
                    payload[6], payload[7],
                ]);
                let number = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
                Ok(Self::WindowExited { id, number })
            }
            MessageKind::CreateWindow => {
                let (program, args) = decode_create_window(&payload).map_err(|_| {
                    ProtocolError::UnexpectedPayload {
                        kind: kind as u8,
                        len: payload.len(),
                    }
                })?;
                Ok(Self::CreateWindow { program, args })
            }
            MessageKind::SelectWindow if payload.len() == 4 => {
                let number = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::SelectWindow { number })
            }
            MessageKind::KillWindow if payload.len() == 4 => {
                let number = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::KillWindow { number })
            }
            MessageKind::WindowList => {
                let list =
                    decode_window_list(&payload).map_err(|_| ProtocolError::UnexpectedPayload {
                        kind: kind as u8,
                        len: payload.len(),
                    })?;
                Ok(Self::WindowList(list))
            }
            MessageKind::WindowTitle => {
                decode_window_title(&payload).map_err(|_| ProtocolError::UnexpectedPayload {
                    kind: kind as u8,
                    len: payload.len(),
                })
            }
            MessageKind::CopyModeRequest => Ok(Self::CopyModeRequest),
            MessageKind::CopyModeData => {
                let lines = decode_copy_mode_data(&payload);
                Ok(Self::CopyModeData(lines))
            }
            MessageKind::PasteRequest => Ok(Self::PasteRequest(payload.to_vec())),
            MessageKind::HardstatusLine => Ok(Self::HardstatusLine(payload.to_vec())),
            MessageKind::CaptionLine => Ok(Self::CaptionLine(payload.to_vec())),
            MessageKind::RenumberWindow if payload.len() == 4 => {
                let bytes: [u8; 4] = payload.try_into().unwrap();
                Ok(Self::RenumberWindow {
                    number: u32::from_be_bytes(bytes),
                })
            }
            MessageKind::Redisplay if payload.is_empty() => Ok(Self::Redisplay),
            MessageKind::RemoveWindow if payload.len() == 4 => {
                let bytes: [u8; 4] = payload.try_into().unwrap();
                Ok(Self::RemoveWindow {
                    number: u32::from_be_bytes(bytes),
                })
            }
            MessageKind::WipeDeadWindows if payload.is_empty() => Ok(Self::WipeDeadWindows),
            MessageKind::Echo => Ok(Self::Echo(payload.to_vec())),
            MessageKind::LogToggle if payload.len() == 1 => Ok(Self::LogToggle {
                enable: payload[0] != 0,
            }),
            MessageKind::LogFile => Ok(Self::LogFile(payload.to_vec())),
            MessageKind::OtherWindow if payload.is_empty() => Ok(Self::OtherWindow),
            MessageKind::MonitorToggle if payload.len() == 1 => Ok(Self::MonitorToggle {
                enable: payload[0] != 0,
            }),
            MessageKind::Activity => Ok(Self::Activity(payload.to_vec())),
            MessageKind::Silence if payload.len() == 2 => {
                let seconds = u16::from_be_bytes([payload[0], payload[1]]);
                Ok(Self::Silence { seconds })
            }
            MessageKind::Bell => Ok(Self::Bell(payload.to_vec())),
            MessageKind::WrapToggle if payload.len() == 1 => Ok(Self::WrapToggle {
                enable: payload[0] != 0,
            }),
            MessageKind::ReadBuf if payload.is_empty() => Ok(Self::ReadBuf),
            MessageKind::WriteBuf => Ok(Self::WriteBuf(payload.to_vec())),
            MessageKind::RemoveBuf if payload.is_empty() => Ok(Self::RemoveBuf),
            MessageKind::Register if !payload.is_empty() => {
                let name = payload[0];
                let data = payload[1..].to_vec();
                Ok(Self::Register { name, data })
            }
            MessageKind::FlowToggle if payload.len() == 1 => Ok(Self::FlowToggle {
                enable: payload[0] != 0,
            }),
            MessageKind::Xoff if payload.is_empty() => Ok(Self::Xoff),
            MessageKind::Xon if payload.is_empty() => Ok(Self::Xon),
            MessageKind::BreakSignal if payload.len() == 2 => {
                let ms = u16::from_be_bytes([payload[0], payload[1]]);
                Ok(Self::BreakSignal { ms })
            }
            MessageKind::WindowInfo => Ok(Self::WindowInfo(payload.to_vec())),
            MessageKind::SearchHistory => Ok(Self::SearchHistory(payload.to_vec())),
            MessageKind::SearchResult => {
                let lines = decode_search_result(&payload);
                Ok(Self::SearchResult(lines))
            }
            MessageKind::Command => Ok(Self::Command(payload.to_vec())),
            MessageKind::Hardcopy if payload.len() >= 4 => {
                let num = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::Hardcopy(num, payload[4..].to_vec()))
            }
            MessageKind::AtWindow if payload.len() >= 4 => {
                let num = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::AtWindow(num, payload[4..].to_vec()))
            }
            MessageKind::SplitVertical => Ok(Self::SplitVertical),
            MessageKind::SplitHorizontal => Ok(Self::SplitHorizontal),
            MessageKind::Suspend => Ok(Self::Suspend),
            MessageKind::RemoveRegion => Ok(Self::RemoveRegion),
            MessageKind::OnlyWindow => Ok(Self::OnlyWindow),
            MessageKind::FocusNext => Ok(Self::FocusNext),
            MessageKind::FocusPrev => Ok(Self::FocusPrev),
            MessageKind::ResizeRegion if payload.len() == 2 => {
                let delta = i16::from_be_bytes([payload[0], payload[1]]);
                Ok(Self::ResizeRegion(delta))
            }
            MessageKind::RegionLayout => {
                let regions = decode_region_layout(&payload);
                Ok(Self::RegionLayout(regions))
            }
            MessageKind::CopyModeMove if payload.len() == 4 => {
                let delta = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(Self::CopyModeMove(delta))
            }
            MessageKind::CopyModeMark => Ok(Self::CopyModeMark),
            MessageKind::CopyModeCopy => Ok(Self::CopyModeCopy),
            MessageKind::CopyModePaste => Ok(Self::CopyModePaste(payload.to_vec())),
            MessageKind::CopyModeCursor if payload.len() >= 10 => {
                let line = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let col = u16::from_be_bytes([payload[4], payload[5]]);
                let total = u32::from_be_bytes([payload[6], payload[7], payload[8], payload[9]]);
                Ok(Self::CopyModeCursor(line, col, total))
            }
            _ => Err(ProtocolError::UnexpectedPayload {
                kind: kind as u8,
                len: payload.len(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum MessageKind {
    Hello = 1,
    HelloAck = 2,
    Shutdown = 3,
    ShutdownAck = 4,
    Attach = 5,
    Detach = 6,
    PtyInput = 7,
    PtyOutput = 8,
    ChildExited = 9,
    Resize = 10,
    Error = 255,
    // Multi-window messages
    CreateWindow = 11,
    WindowCreated = 12,
    SelectWindow = 13,
    WindowSelected = 14,
    KillWindow = 15,
    WindowExited = 16,
    WindowList = 17,
    NextWindow = 18,
    PrevWindow = 19,
    WindowTitle = 20,
    CopyModeRequest = 21,
    CopyModeData = 22,
    PasteRequest = 23,
    HardstatusLine = 24,
    RenumberWindow = 25,
    Redisplay = 26,
    RemoveWindow = 27,
    WipeDeadWindows = 28,
    Echo = 29,
    LogToggle = 30,
    LogFile = 31,
    OtherWindow = 32,
    MonitorToggle = 33,
    Activity = 34,
    Silence = 35,
    Bell = 36,
    WrapToggle = 37,
    ReadBuf = 38,
    WriteBuf = 39,
    RemoveBuf = 40,
    Register = 41,
    FlowToggle = 42,
    Xoff = 43,
    Xon = 44,
    BreakSignal = 45,
    WindowInfo = 46,
    SearchHistory = 47,
    SearchResult = 48,
    Command = 49,
    Hardcopy = 50,
    AtWindow = 51,
    SplitVertical = 52,
    RemoveRegion = 53,
    OnlyWindow = 54,
    FocusNext = 55,
    FocusPrev = 56,
    ResizeRegion = 57,
    RegionLayout = 58,
    CopyModeMove = 59,
    CopyModeMark = 60,
    CopyModeCopy = 61,
    CopyModePaste = 62,
    CopyModeCursor = 63,
    CaptionLine = 64,
    SplitHorizontal = 65,
    Suspend = 66,
}

impl TryFrom<u8> for MessageKind {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::HelloAck),
            3 => Ok(Self::Shutdown),
            4 => Ok(Self::ShutdownAck),
            5 => Ok(Self::Attach),
            6 => Ok(Self::Detach),
            7 => Ok(Self::PtyInput),
            8 => Ok(Self::PtyOutput),
            9 => Ok(Self::ChildExited),
            10 => Ok(Self::Resize),
            11 => Ok(Self::CreateWindow),
            12 => Ok(Self::WindowCreated),
            13 => Ok(Self::SelectWindow),
            14 => Ok(Self::WindowSelected),
            15 => Ok(Self::KillWindow),
            16 => Ok(Self::WindowExited),
            17 => Ok(Self::WindowList),
            18 => Ok(Self::NextWindow),
            19 => Ok(Self::PrevWindow),
            20 => Ok(Self::WindowTitle),
            21 => Ok(Self::CopyModeRequest),
            22 => Ok(Self::CopyModeData),
            23 => Ok(Self::PasteRequest),
            24 => Ok(Self::HardstatusLine),
            25 => Ok(Self::RenumberWindow),
            26 => Ok(Self::Redisplay),
            27 => Ok(Self::RemoveWindow),
            28 => Ok(Self::WipeDeadWindows),
            29 => Ok(Self::Echo),
            30 => Ok(Self::LogToggle),
            31 => Ok(Self::LogFile),
            32 => Ok(Self::OtherWindow),
            33 => Ok(Self::MonitorToggle),
            34 => Ok(Self::Activity),
            35 => Ok(Self::Silence),
            36 => Ok(Self::Bell),
            37 => Ok(Self::WrapToggle),
            38 => Ok(Self::ReadBuf),
            39 => Ok(Self::WriteBuf),
            40 => Ok(Self::RemoveBuf),
            41 => Ok(Self::Register),
            42 => Ok(Self::FlowToggle),
            43 => Ok(Self::Xoff),
            44 => Ok(Self::Xon),
            45 => Ok(Self::BreakSignal),
            46 => Ok(Self::WindowInfo),
            47 => Ok(Self::SearchHistory),
            48 => Ok(Self::SearchResult),
            49 => Ok(Self::Command),
            50 => Ok(Self::Hardcopy),
            51 => Ok(Self::AtWindow),
            52 => Ok(Self::SplitVertical),
            53 => Ok(Self::RemoveRegion),
            54 => Ok(Self::OnlyWindow),
            55 => Ok(Self::FocusNext),
            56 => Ok(Self::FocusPrev),
            57 => Ok(Self::ResizeRegion),
            58 => Ok(Self::RegionLayout),
            59 => Ok(Self::CopyModeMove),
            60 => Ok(Self::CopyModeMark),
            61 => Ok(Self::CopyModeCopy),
            62 => Ok(Self::CopyModePaste),
            63 => Ok(Self::CopyModeCursor),
            64 => Ok(Self::CaptionLine),
            255 => Ok(Self::Error),
            value => Err(ProtocolError::UnknownMessage(value)),
        }
    }
}

fn checked_payload(payload: &[u8]) -> Result<&[u8], ProtocolError> {
    if payload.len() > MAX_PAYLOAD_LEN as usize {
        return Err(ProtocolError::PayloadTooLarge(payload.len()));
    }
    Ok(payload)
}

fn encode_create_window(program: &[u8], args: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = Vec::new();
    // program length: u16
    buf.extend_from_slice(&(program.len() as u16).to_be_bytes());
    buf.extend_from_slice(program);
    // number of args: u16
    buf.extend_from_slice(&(args.len() as u16).to_be_bytes());
    for arg in args {
        buf.extend_from_slice(&(arg.len() as u16).to_be_bytes());
        buf.extend_from_slice(arg);
    }
    buf
}

fn decode_create_window(payload: &[u8]) -> Result<(Vec<u8>, Vec<Vec<u8>>), &'static str> {
    if payload.len() < 2 {
        return Err("too short for program length");
    }
    let prog_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let mut pos = 2;
    if pos + prog_len > payload.len() {
        return Err("program bytes truncated");
    }
    let program = payload[pos..pos + prog_len].to_vec();
    pos += prog_len;
    if pos + 2 > payload.len() {
        return Err("too short for arg count");
    }
    let num_args = u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize;
    pos += 2;
    let mut args = Vec::with_capacity(num_args);
    for _ in 0..num_args {
        if pos + 2 > payload.len() {
            return Err("too short for arg length");
        }
        let arg_len = u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize;
        pos += 2;
        if pos + arg_len > payload.len() {
            return Err("arg bytes truncated");
        }
        args.push(payload[pos..pos + arg_len].to_vec());
        pos += arg_len;
    }
    Ok((program, args))
}

fn encode_window_list(list: &[WindowInfoMsg]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(list.len() as u16).to_be_bytes());
    for w in list {
        buf.extend_from_slice(&w.number.to_be_bytes());
        buf.push(w.flags);
        // group: length-prefixed (u16), 0-length means None
        if let Some(ref g) = w.group {
            buf.extend_from_slice(&(g.len() as u16).to_be_bytes());
            buf.extend_from_slice(g);
        } else {
            buf.extend_from_slice(&0u16.to_be_bytes());
        }
        buf.extend_from_slice(&(w.title.len() as u16).to_be_bytes());
        buf.extend_from_slice(&w.title);
    }
    buf
}

fn decode_window_list(payload: &[u8]) -> Result<Vec<WindowInfoMsg>, &'static str> {
    if payload.len() < 2 {
        return Err("too short for window count");
    }
    let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let mut pos = 2;
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 7 > payload.len() {
            return Err("window entry truncated");
        }
        let number = u32::from_be_bytes([
            payload[pos],
            payload[pos + 1],
            payload[pos + 2],
            payload[pos + 3],
        ]);
        let flags = payload[pos + 4];
        pos += 5;
        if pos + 2 > payload.len() {
            return Err("group length truncated");
        }
        let group_len = u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize;
        pos += 2;
        let group = if group_len == 0 {
            None
        } else {
            if pos + group_len > payload.len() {
                return Err("group bytes truncated");
            }
            let g = payload[pos..pos + group_len].to_vec();
            pos += group_len;
            Some(g)
        };
        if pos + 2 > payload.len() {
            return Err("title length truncated");
        }
        let title_len = u16::from_be_bytes([payload[pos], payload[pos + 1]]) as usize;
        pos += 2;
        if pos + title_len > payload.len() {
            return Err("title bytes truncated");
        }
        let title = payload[pos..pos + title_len].to_vec();
        pos += title_len;
        list.push(WindowInfoMsg {
            number,
            flags,
            title,
            group,
        });
    }
    Ok(list)
}

fn encode_window_title(number: u32, title: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&number.to_be_bytes());
    buf.extend_from_slice(&(title.len() as u16).to_be_bytes());
    buf.extend_from_slice(title);
    buf
}

fn decode_window_title(payload: &[u8]) -> Result<Message, ProtocolError> {
    if payload.len() < 6 {
        return Err(ProtocolError::UnexpectedPayload {
            kind: MessageKind::WindowTitle as u8,
            len: payload.len(),
        });
    }
    let number = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let title_len = u16::from_be_bytes([payload[4], payload[5]]) as usize;
    if 6 + title_len > payload.len() {
        return Err(ProtocolError::UnexpectedPayload {
            kind: MessageKind::WindowTitle as u8,
            len: payload.len(),
        });
    }
    let title = payload[6..6 + title_len].to_vec();
    Ok(Message::WindowTitle { number, title })
}

fn encode_copy_mode_data(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(lines.len() as u32).to_be_bytes());
    for line in lines {
        buf.extend_from_slice(&(line.len() as u16).to_be_bytes());
        buf.extend_from_slice(line);
    }
    buf
}

fn decode_copy_mode_data(payload: &[u8]) -> Vec<Vec<u8>> {
    if payload.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let mut lines = Vec::with_capacity(count.min(10000));
    let mut offset = 4usize;
    for _ in 0..count {
        if offset + 2 > payload.len() {
            break;
        }
        let len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
        offset += 2;
        if offset + len > payload.len() {
            break;
        }
        lines.push(payload[offset..offset + len].to_vec());
        offset += len;
    }
    lines
}

fn encode_register(name: u8, data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + data.len());
    buf.push(name);
    buf.extend_from_slice(data);
    buf
}

fn encode_search_result(lines: &[u32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(2 + lines.len() * 4);
    buf.extend_from_slice(&(lines.len() as u16).to_be_bytes());
    for line in lines {
        buf.extend_from_slice(&line.to_be_bytes());
    }
    buf
}

fn decode_search_result(payload: &[u8]) -> Vec<u32> {
    if payload.len() < 2 {
        return Vec::new();
    }
    let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let mut result = Vec::with_capacity(count.min(10000));
    let mut offset = 2usize;
    for _ in 0..count {
        if offset + 4 > payload.len() {
            break;
        }
        let line = u32::from_be_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        result.push(line);
        offset += 4;
    }
    result
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    BadMagic,
    UnsupportedVersion(u16),
    UnknownMessage(u8),
    PayloadTooLarge(usize),
    UnexpectedPayload { kind: u8, len: usize },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "protocol I/O error: {error}"),
            Self::BadMagic => formatter.write_str("invalid protocol magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::UnknownMessage(kind) => write!(formatter, "unknown protocol message {kind}"),
            Self::PayloadTooLarge(len) => write!(formatter, "protocol payload too large: {len}"),
            Self::UnexpectedPayload { kind, len } => {
                write!(
                    formatter,
                    "message {kind} does not accept payload length {len}"
                )
            }
        }
    }
}

impl Error for ProtocolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

fn encode_region_layout(regions: &[(u32, u16, u16, u16, u16, bool)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(regions.len() * 13);
    for (num, top, height, left, width, focused) in regions {
        buf.extend_from_slice(&num.to_be_bytes());
        buf.extend_from_slice(&top.to_be_bytes());
        buf.extend_from_slice(&height.to_be_bytes());
        buf.extend_from_slice(&left.to_be_bytes());
        buf.extend_from_slice(&width.to_be_bytes());
        buf.push(if *focused { 1 } else { 0 });
    }
    buf
}

fn decode_region_layout(payload: &[u8]) -> Vec<(u32, u16, u16, u16, u16, bool)> {
    payload
        .chunks_exact(13)
        .map(|c| {
            let num = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
            let top = u16::from_be_bytes([c[4], c[5]]);
            let height = u16::from_be_bytes([c[6], c[7]]);
            let left = u16::from_be_bytes([c[8], c[9]]);
            let width = u16::from_be_bytes([c[10], c[11]]);
            let focused = c[12] != 0;
            (num, top, height, left, width, focused)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips() {
        let mut bytes = Vec::new();
        Message::Hello.write_to(&mut bytes).expect("encode hello");

        assert_eq!(
            Message::read_from(&mut bytes.as_slice()).expect("decode hello"),
            Message::Hello
        );
    }

    #[test]
    fn shutdown_round_trips() {
        let mut bytes = Vec::new();
        Message::Shutdown
            .write_to(&mut bytes)
            .expect("encode shutdown");

        assert_eq!(
            Message::read_from(&mut bytes.as_slice()).expect("decode shutdown"),
            Message::Shutdown
        );
    }

    #[test]
    fn pty_payload_round_trips() {
        let mut bytes = Vec::new();
        Message::PtyOutput(b"ready".to_vec())
            .write_to(&mut bytes)
            .expect("encode pty output");

        assert_eq!(
            Message::read_from(&mut bytes.as_slice()).expect("decode pty output"),
            Message::PtyOutput(b"ready".to_vec())
        );
    }

    #[test]
    fn resize_round_trips() {
        let mut bytes = Vec::new();
        Message::Resize {
            columns: 100,
            rows: 40,
        }
        .write_to(&mut bytes)
        .expect("encode resize");

        assert_eq!(
            Message::read_from(&mut bytes.as_slice()).expect("decode resize"),
            Message::Resize {
                columns: 100,
                rows: 40,
            }
        );
    }

    #[test]
    fn bad_magic_is_rejected() {
        let bytes = [0_u8; HEADER_LEN];

        assert!(matches!(
            Message::read_from(&mut bytes.as_slice()),
            Err(ProtocolError::BadMagic)
        ));
    }

    #[test]
    fn unexpected_payload_is_rejected_for_fixed_message() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        bytes.push(MessageKind::Hello as u8);
        bytes.extend_from_slice(&1_u32.to_be_bytes());
        bytes.push(b'x');

        assert!(matches!(
            Message::read_from(&mut bytes.as_slice()),
            Err(ProtocolError::UnexpectedPayload { .. })
        ));
    }
}
