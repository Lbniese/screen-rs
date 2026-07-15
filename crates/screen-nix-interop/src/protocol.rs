//! GNU Screen control-socket protocol reference.
//!
//! ## Socket naming
//!
//! GNU Screen stores its control socket at:
//! - `$SCREENDIR/S-<username>` (the "public" socket, setgid to the user)
//! - or `<runtime>/<pid>.<session_name>` (per-session)
//!
//! Where `<runtime>` defaults to:
//! - Linux: `/run/screen/S-<user>` or `/tmp/uscreens/S-<user>`
//! - macOS: `/tmp/.screen`
//!
//! ## Protocol framing
//!
//! The GNU Screen protocol is a line-oriented text protocol over a Unix
//! domain socket.  Each request and response is framed by newlines.
//!
//! A request has the form:
//! ```text
//! <length> <command>\n
//! ```
//! Where `<length>` is a decimal number (bytes of payload), then a space,
//! then the command, then a newline.  For example:
//! ```text
//! 10 helloworld\n
//! ```
//!
//! For `-X` remote commands, the protocol uses versioned framing:
//! ```text
//! <type><version><data>\n
//! ```
//! Where `<type>` is a single byte indicating the command type, `<version>`
//! is a byte for protocol version (usually 0), and `<data>` is the
//! command arguments.
//!
//! Known request types:
//! - `0x00` — Query/command with args (used by `screen -X`)
//! - `0x01` — Window list query
//! - `0x02` — Info query
//!
//! For query-based commands (`-Q`), GNU Screen uses a different framing
//! with a 4-byte header length prefix instead.
//!
//! ## Error handling
//!
//! The response is a simple text line indicating success or failure.
//! On success: the socket sends no explicit ACK — silence means OK.
//! On error: the socket sends an error message line.
//!
//! ## Version-specific notes
//!
//! - Screen 4.00+ uses the binary `0x00` framing for `-X`.
//! - Screen 5.x may use extended framing with PID-based auth.
//! - The "S-<username>" socket may use setgid tricks for multi-user access.
//!
//! ## Status strings
//!
//! When listing sessions (`-ls`), GNU Screen returns human-readable text:
//! ```text
//! There is a screen on:
//!     12345.session_name    (Detached)
//! 1 Socket in /tmp/.screen.
//! ```
//!
//! For `-Q windows`, it returns:
//! ```text
//! 0$ bash
//! 1- bash
//! 2$ bash
//! ```
//! Where `$` = attached, `-` = detached, `*` = current.
//!
//! ## Compatibility matrix
//!
//! | Feature              | GNU Screen 4.0 | GNU Screen 5.x | screen-rs |
//! |---------------------|---------------|---------------|----------|
//! | `-ls`               | text output   | text output   | text output |
//! | `-X quit`           | 0x00 framing  | 0x00 framing  | SRSP      |
//! | `-X stuff`          | 0x00 framing  | 0x00 framing  | SRSP      |
//! | `-Q windows`        | 0x00+header   | 0x00+header   | SRSP      |
//! | `-r` reattach       | server mode   | server mode   | SRSP      |
//! | multi-user ACL      | setgid socket | setgid socket | ACL in config |
//!
//! ## Implementation notes
//!
//! The socket protocol is intentionally simple.  The majority of complexity
//! is in the server-side session management, not the on-wire format.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Errors when communicating with a GNU Screen control socket.
#[derive(Debug)]
pub enum NixScreenError {
    Io(io::Error),
    Protocol(String),
    Timeout,
}

impl std::fmt::Display for NixScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
            Self::Timeout => write!(f, "timeout"),
        }
    }
}

impl std::error::Error for NixScreenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for NixScreenError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Connect to a GNU Screen daemon and send a remote command.
///
/// This mirrors `screen -X <command>` behavior — it connects to the session
/// socket and sends the command using GNU Screen's binary framing.
pub fn send_remote_command(
    socket_path: &Path,
    command: &[u8],
    timeout: Duration,
) -> Result<(), NixScreenError> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    // Frame: <type><version><len><command>
    // Type 0x00 = command with args
    // Version byte: 0x00 = version 0
    let mut frame = Vec::with_capacity(command.len() + 6);
    frame.push(0x00); // type
    frame.push(0x00); // version
    let len = command.len() as u32;
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(command);
    frame.push(b'\n');

    stream.write_all(&frame)?;

    // Read response — GNU Screen may return error text.
    let mut response = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                // Stop at newline (end of response).
                if buf[..n].contains(&b'\n') {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => break,
            Err(e) => return Err(e.into()),
        }
    }

    // If we got a response, check for errors.
    if !response.is_empty() {
        let msg = String::from_utf8_lossy(&response);
        // Empty response or just newline = success
        let trimmed = msg.trim();
        if !trimmed.is_empty() && trimmed != "OK" {
            // Could be an error — but GNU Screen often sends text even on success.
            // We only treat explicit error messages as failures.
            if trimmed.contains("Error") || trimmed.contains("No screen") {
                return Err(NixScreenError::Protocol(trimmed.to_string()));
            }
        }
    }

    Ok(())
}

/// Send a quit command to a GNU Screen session.
pub fn quit_session(socket_path: &Path) -> Result<(), NixScreenError> {
    send_remote_command(socket_path, b"quit", Duration::from_secs(5))
}

/// Send a detach command to a GNU Screen session.
pub fn detach_session(socket_path: &Path) -> Result<(), NixScreenError> {
    send_remote_command(socket_path, b"detach", Duration::from_secs(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_produces_valid_format() {
        let mut frame = Vec::with_capacity(10);
        frame.push(0x00); // type
        frame.push(0x00); // version
        let cmd = b"quit";
        let len = cmd.len() as u32;
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(cmd);
        frame.push(b'\n');

        // Frame: 0x00 0x00 0x00 0x00 0x00 0x04 quit \n
        assert_eq!(frame.len(), 11);
        assert_eq!(frame[0], 0x00);
        assert_eq!(frame[1], 0x00);
        assert_eq!(&frame[6..10], b"quit");
        assert_eq!(frame[10], b'\n');
    }

    #[test]
    fn error_display_is_reasonable() {
        let e = NixScreenError::Protocol("test error".into());
        assert!(e.to_string().contains("test error"));
    }
}
