#![forbid(unsafe_code)]

//! GNU Screen control-socket interoperability layer.
//!
//! GNU Screen uses a control socket at `$SCREENDIR/S-<username>` (or a
//! per-session socket at `<runtime>/<pid>.<name>`) that accepts a simple
//! binary protocol for remote commands (`screen -X` and friends).
//!
//! This crate provides:
//! - **Socket discovery**: enumerates screen sessions (both GNU Screen and
//!   screen-rs) and returns structured metadata.
//! - **Protocol client**: minimal GNU Screen protocol client for sending
//!   commands (`-X`) to a GNU Screen daemon.
//! - **Protocol docs**: reference documentation for the protocol framing and
//!   message types.

pub mod discovery;
pub mod protocol;

/// Screen implementation variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    /// Our implementation (screen-rs, custom SRSP protocol).
    ScreenRs,
    /// GNU Screen (original S-<user> protocol).
    GnuScreen,
}

/// A discovered screen session.
#[derive(Debug, Clone)]
pub struct DiscoveredSession {
    /// Human-readable session name.
    pub name: String,
    /// PID that owns the session.
    pub pid: u32,
    /// Path to the control socket.
    pub socket_path: std::path::PathBuf,
    /// Whether this is a screen-rs or GNU Screen session.
    pub kind: ScreenKind,
    /// Whether the session appears to be attached or detached.
    pub attached: bool,
}
