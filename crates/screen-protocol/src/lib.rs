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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfoMsg {
    pub number: u32,
    pub flags: u8,
    pub title: Vec<u8>,
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
        if pos + 5 > payload.len() {
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
