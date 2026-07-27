use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::DaemonError;

pub(crate) struct SocketCleanup {
    path: PathBuf,
}

impl SocketCleanup {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn ensure_parent_exists(path: &Path) -> Result<(), DaemonError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|source| DaemonError::Io {
        path: parent.to_owned(),
        source,
    })
}

pub(crate) fn restrict_socket_permissions(path: &Path) -> Result<(), DaemonError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| DaemonError::Io {
        path: path.to_owned(),
        source,
    })
}

pub(crate) fn reject_existing_socket_path(path: &Path) -> Result<(), DaemonError> {
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

pub(crate) fn sty_value(socket_path: &Path) -> OsString {
    socket_path
        .file_name()
        .unwrap_or_else(|| OsStr::new("screen-rs"))
        .to_owned()
}

pub(crate) fn open_log_file(path: Option<&Path>) -> Result<Option<File>, DaemonError> {
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
