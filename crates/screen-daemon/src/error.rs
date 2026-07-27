use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use screen_protocol::ProtocolError;
use screen_pty::PtyError;

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
