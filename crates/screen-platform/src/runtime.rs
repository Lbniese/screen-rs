use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

unsafe extern "C" {
    fn geteuid() -> u32;
}

pub fn current_effective_uid() -> u32 {
    // SAFETY: POSIX geteuid has no preconditions, does not take pointers or file descriptors,
    // and cannot fail. The returned uid is copied by value.
    unsafe { geteuid() }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDirectory {
    path: PathBuf,
}

impl RuntimeDirectory {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, RuntimeDirectoryError> {
        Self::open_for_owner(path, current_effective_uid())
    }

    pub fn open_for_owner(
        path: impl Into<PathBuf>,
        expected_uid: u32,
    ) -> Result<Self, RuntimeDirectoryError> {
        let path = path.into();
        validate_directory(&path, expected_uid)?;
        Ok(Self { path })
    }

    pub fn create_private(path: impl Into<PathBuf>) -> Result<Self, RuntimeDirectoryError> {
        let path = path.into();
        fs::create_dir_all(&path).map_err(|source| RuntimeDirectoryError::Io {
            path: path.clone(),
            source,
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            RuntimeDirectoryError::Io {
                path: path.clone(),
                source,
            }
        })?;
        Self::open(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn session_socket_path(&self, session_name: &OsStr) -> Result<PathBuf, SessionNameError> {
        validate_session_name(session_name)?;
        Ok(self.path.join(session_name))
    }

    pub fn classify_session_socket(
        &self,
        session_name: &OsStr,
    ) -> Result<SocketPathStatus, RuntimeDirectoryError> {
        let path = self.session_socket_path(session_name).map_err(|source| {
            RuntimeDirectoryError::InvalidSessionName {
                name: session_name.to_owned(),
                source,
            }
        })?;
        classify_socket_path(path)
    }
}

#[derive(Debug)]
pub enum RuntimeDirectoryError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    NotDirectory {
        path: PathBuf,
    },
    Symlink {
        path: PathBuf,
    },
    WrongOwner {
        path: PathBuf,
        expected_uid: u32,
        actual_uid: u32,
    },
    UnsafePermissions {
        path: PathBuf,
        mode: u32,
    },
    InvalidSessionName {
        name: OsString,
        source: SessionNameError,
    },
}

impl fmt::Display for RuntimeDirectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::NotDirectory { path } => {
                write!(formatter, "{} is not a directory", path.display())
            }
            Self::Symlink { path } => {
                write!(formatter, "{} is a symlink and is not safe", path.display())
            }
            Self::WrongOwner {
                path,
                expected_uid,
                actual_uid,
            } => write!(
                formatter,
                "{} is owned by uid {actual_uid}, expected uid {expected_uid}",
                path.display()
            ),
            Self::UnsafePermissions { path, mode } => write!(
                formatter,
                "{} has unsafe permissions {:o}; group/other write bits must be clear",
                path.display(),
                mode & 0o777
            ),
            Self::InvalidSessionName { name, source } => {
                write!(formatter, "invalid session name {:?}: {source}", name)
            }
        }
    }
}

impl Error for RuntimeDirectoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidSessionName { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionNameError {
    Empty,
    ContainsSlash,
    ContainsNul,
}

impl fmt::Display for SessionNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("empty names are not valid socket names"),
            Self::ContainsSlash => formatter.write_str("slash is not valid in a socket name"),
            Self::ContainsNul => formatter.write_str("nul is not valid in a socket name"),
        }
    }
}

impl Error for SessionNameError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketPathStatus {
    Missing,
    ActiveSocket,
    StaleSocket,
    RegularFile,
    Directory,
    Other,
}

pub fn classify_socket_path(
    path: impl Into<PathBuf>,
) -> Result<SocketPathStatus, RuntimeDirectoryError> {
    let path = path.into();
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(SocketPathStatus::Missing);
        }
        Err(source) => return Err(RuntimeDirectoryError::Io { path, source }),
    };

    let file_type = metadata.file_type();
    if file_type.is_socket() {
        return Ok(match UnixStream::connect(&path) {
            Ok(_stream) => SocketPathStatus::ActiveSocket,
            Err(_error) => SocketPathStatus::StaleSocket,
        });
    }
    if file_type.is_file() {
        return Ok(SocketPathStatus::RegularFile);
    }
    if file_type.is_dir() {
        return Ok(SocketPathStatus::Directory);
    }
    Ok(SocketPathStatus::Other)
}

fn validate_directory(path: &Path, expected_uid: u32) -> Result<(), RuntimeDirectoryError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeDirectoryError::Io {
        path: path.to_owned(),
        source,
    })?;

    if metadata.file_type().is_symlink() {
        return Err(RuntimeDirectoryError::Symlink {
            path: path.to_owned(),
        });
    }

    if !metadata.file_type().is_dir() {
        return Err(RuntimeDirectoryError::NotDirectory {
            path: path.to_owned(),
        });
    }

    let actual_uid = metadata.uid();
    if actual_uid != expected_uid {
        return Err(RuntimeDirectoryError::WrongOwner {
            path: path.to_owned(),
            expected_uid,
            actual_uid,
        });
    }

    let mode = metadata.permissions().mode();
    if mode & 0o022 != 0 {
        return Err(RuntimeDirectoryError::UnsafePermissions {
            path: path.to_owned(),
            mode,
        });
    }

    Ok(())
}

fn validate_session_name(name: &OsStr) -> Result<(), SessionNameError> {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return Err(SessionNameError::Empty);
    }
    if bytes.contains(&b'/') {
        return Err(SessionNameError::ContainsSlash);
    }
    if bytes.contains(&0) {
        return Err(SessionNameError::ContainsNul);
    }
    Ok(())
}
