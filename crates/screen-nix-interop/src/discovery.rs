//! Socket discovery for GNU Screen and screen-rs sessions.
//!
//! GNU Screen session sockets live in `$SCREENDIR` (or `/tmp/uscreens/S-<user>`
//! by default on Linux, `/tmp/.screen` on some systems, and
//! `/var/run/screen/S-<user>` on others).  Each session is represented as a
//! file named `<pid>.<session_name>` inside that directory.
//!
//! screen-rs uses the same `$SCREENDIR` convention but with its own SRSP
//! protocol.  This module enumerates both kinds.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{DiscoveredSession, ScreenKind};

/// Maximum session name length (GNU Screen limit: 80 chars including NUL).
const MAX_SESSION_NAME_LEN: usize = 79;

/// Enumerate all screen sessions visible to this user.
///
/// Scans `$SCREENDIR` (falling back to the platform default) and returns
/// structured metadata for every discoverable session — both GNU Screen and
/// screen-rs.
pub fn discover_sessions() -> Vec<DiscoveredSession> {
    let dir = screendir();
    let mut sessions = HashMap::new();

    if !dir.exists() || !dir.is_dir() {
        return Vec::new();
    }

    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        // GNU Screen session: <pid>.<session_name>
        // screen-rs session:    <pid>.<session_name>  (same convention)
        if let Some(session) = parse_session_entry(&path, name) {
            sessions.insert(session.name.clone(), session);
        }
    }

    let mut result: Vec<_> = sessions.into_values().collect();
    // Stable sort by name for deterministic listing.
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

/// Probe whether a specific session exists.
pub fn session_exists(session_name: &str) -> bool {
    discover_sessions().iter().any(|s| s.name == session_name)
}

/// Get the control socket path for a named session, if it exists.
pub fn session_socket(session_name: &str) -> Option<PathBuf> {
    discover_sessions()
        .into_iter()
        .find(|s| s.name == session_name)
        .map(|s| s.socket_path)
}

// ── internal ──────────────────────────────────────────────────────────────

fn screendir() -> PathBuf {
    // Respect SCREENDIR override (shared by both GNU Screen and screen-rs).
    if let Ok(dir) = std::env::var("SCREENDIR") {
        return PathBuf::from(dir);
    }

    // Platform defaults — matching GNU Screen's `Makefile.in` logic.
    #[cfg(target_os = "linux")]
    let user = std::env::var("USER").unwrap_or_default();

    #[cfg(target_os = "linux")]
    {
        // GNU Screen on Linux defaults to under /run/screen/S-<user> or
        // /tmp/uscreens/S-<user> depending on build flags.
        let modern = PathBuf::from(format!("/run/screen/S-{user}"));
        if modern.exists() {
            return modern;
        }
        let legacy = PathBuf::from(format!("/tmp/uscreens/S-{user}"));
        return legacy;
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: /tmp/.screen (some builds) or /var/run/screen.
        // Homebrew GNU Screen uses /tmp/.screen.
        let tmp_screen = PathBuf::from("/tmp/.screen");
        if tmp_screen.exists() {
            return tmp_screen;
        }
        std::env::temp_dir()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        std::env::temp_dir()
    }
}

/// Parse a filename like `12345.mysession` or `67890.other_session` into
/// a `DiscoveredSession`, probing the socket to determine which kind.
fn parse_session_entry(path: &Path, name: &str) -> Option<DiscoveredSession> {
    // Must match: <digits>.<non-empty-session-name>
    let dot_pos = name.find('.')?;
    let pid_str = &name[..dot_pos];
    let session_name = &name[dot_pos + 1..];

    if pid_str.is_empty()
        || !pid_str.bytes().all(|b| b.is_ascii_digit())
        || session_name.is_empty()
        || session_name.len() > MAX_SESSION_NAME_LEN
    {
        return None;
    }

    let pid: u32 = pid_str.parse().ok()?;

    // Determine attached/detached by trying to connect to the socket.
    // If we can connect, the session might be unattended.
    let attached = probe_socket_liveness(path);

    // Determine kind by probing the socket protocol.
    let kind = detect_socket_kind(path);

    Some(DiscoveredSession {
        name: session_name.to_owned(),
        pid,
        socket_path: path.to_path_buf(),
        kind,
        attached,
    })
}

/// Quick probe: try connecting to the socket. If it succeeds, the daemon is
/// alive (session is detached or has at least one attached client).
fn probe_socket_liveness(socket_path: &Path) -> bool {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(socket_path).is_ok()
}

/// Determine whether a socket speaks the screen-rs (SRSP) or GNU Screen
/// protocol.
///
/// Strategy: connect, read the first few bytes.  screen-rs sockets begin
/// with the SRSP magic `b"SRSP"`.  GNU Screen sockets use a different
/// framing — bytes `0x00 ...` or `/dev/` prefix.
fn detect_socket_kind(socket_path: &Path) -> ScreenKind {
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    let mut stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return ScreenKind::GnuScreen, // default fallback
    };

    // Set a short read timeout.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(100)));

    let mut magic = [0u8; 4];
    match stream.read_exact(&mut magic) {
        Ok(()) if &magic == b"SRSP" => ScreenKind::ScreenRs,
        _ => ScreenKind::GnuScreen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_session_entry() {
        let _tmp = std::env::temp_dir().join("test-12345.session_name");
        // We can't create a real socket easily in a test, so just test parsing
        // without probing the socket.
        assert!(
            Path::new("/tmp/12345.testsession")
                .file_name()
                .and_then(OsStr::to_str)
                .and_then(|n| parse_session_entry(&PathBuf::from(format!("/tmp/{n}")), n))
                .is_some()
        );
    }

    #[test]
    fn reject_invalid_session_entries() {
        // No dot
        assert!(parse_session_entry(Path::new("/tmp/nosession"), "nosession").is_none());
        // Empty session name
        assert!(parse_session_entry(Path::new("/tmp/12345."), "12345.").is_none());
        // Non-digit pid
        assert!(parse_session_entry(Path::new("/tmp/abc.test"), "abc.test").is_none());
    }

    #[test]
    fn discover_sessions_does_not_panic() {
        // Even when SCREENDIR is unset or points somewhere bogus.
        let _ = discover_sessions();
    }
}
