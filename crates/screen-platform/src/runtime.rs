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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Return a unique temporary directory path for a test.
    /// The directory is removed first if it already exists.
    fn test_dir(label: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("screen-rust-test-{}-{}", label, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    /// Create a directory with safe (0o700) permissions for testing.
    fn create_safe_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    // ------------------------------------------------------------------
    // current_effective_uid
    // ------------------------------------------------------------------

    #[test]
    fn test_current_effective_uid_returns_valid_value() {
        let uid = current_effective_uid();
        // geteuid never fails; any u32 is a valid uid_t on the platform.
        // Just verify it returns something (no panic) and makes sense.
        assert!(uid < 100_000, "unexpectedly large UID: {uid}");
    }

    // ------------------------------------------------------------------
    // RuntimeDirectory::create_private
    // ------------------------------------------------------------------

    #[test]
    fn test_create_private_directory() {
        let dir = test_dir("create_private");
        let result = RuntimeDirectory::create_private(&dir);
        assert!(result.is_ok(), "create_private failed: {:?}", result.err());
        let runtime = result.unwrap();
        assert_eq!(runtime.path(), &dir);

        // Verify the directory exists with 0o700 permissions
        let meta = fs::symlink_metadata(&dir).unwrap();
        assert!(meta.file_type().is_dir());
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "expected 0o700, got {mode:o}");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_create_private_directory_is_usable_via_open() {
        let dir = test_dir("create_private_usable");
        let _runtime = RuntimeDirectory::create_private(&dir).unwrap();
        // A second open should also succeed
        let reopened = RuntimeDirectory::open(&dir);
        assert!(
            reopened.is_ok(),
            "reopen after create_private failed: {:?}",
            reopened.err()
        );
        fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------
    // RuntimeDirectory::open / open_for_owner
    // ------------------------------------------------------------------

    #[test]
    fn test_open_own_directory() {
        let dir = test_dir("open_own");
        create_safe_dir(&dir);

        let result = RuntimeDirectory::open(&dir);
        assert!(
            result.is_ok(),
            "open of own dir should succeed: {:?}",
            result.err()
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_open_for_owner_wrong_uid() {
        let dir = test_dir("open_wrong_uid");
        create_safe_dir(&dir);

        let uid = current_effective_uid();
        let wrong_uid = if uid == 0 { 1 } else { 0 };

        let result = RuntimeDirectory::open_for_owner(&dir, wrong_uid);
        match result {
            Err(RuntimeDirectoryError::WrongOwner {
                path,
                expected_uid,
                actual_uid,
            }) => {
                assert_eq!(path, dir);
                assert_eq!(expected_uid, wrong_uid);
                assert_eq!(actual_uid, uid);
            }
            other => panic!("expected WrongOwner, got: {:?}", other),
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_open_nonexistent_directory() {
        let dir = test_dir("open_nonexistent");
        let result = RuntimeDirectory::open(&dir);
        assert!(result.is_err());
        assert!(matches!(result, Err(RuntimeDirectoryError::Io { .. })));
    }

    // ------------------------------------------------------------------
    // NotDirectory / Symlink detection
    // ------------------------------------------------------------------

    #[test]
    fn test_not_directory_rejected() {
        let dir = test_dir("not_dir");
        create_safe_dir(&dir);
        let file_path = dir.join("regular.txt");
        fs::write(&file_path, b"content").unwrap();

        let result = RuntimeDirectory::open(&file_path);
        match result {
            Err(RuntimeDirectoryError::NotDirectory { path }) => {
                assert_eq!(path, file_path);
            }
            other => panic!("expected NotDirectory, got: {:?}", other),
        }
        fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------
    // RuntimeDirectory::session_socket_path
    // ------------------------------------------------------------------

    #[test]
    fn test_session_socket_path_valid_name() {
        let dir = test_dir("sock_valid");
        create_safe_dir(&dir);
        let runtime = RuntimeDirectory::open(&dir).unwrap();

        let path = runtime
            .session_socket_path(OsStr::new("my-session"))
            .unwrap();
        assert_eq!(path, dir.join("my-session"));

        let path2 = runtime.session_socket_path(OsStr::new("a.b-c_d")).unwrap();
        assert_eq!(path2, dir.join("a.b-c_d"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_session_socket_path_empty_rejected() {
        let dir = test_dir("sock_empty");
        create_safe_dir(&dir);
        let runtime = RuntimeDirectory::open(&dir).unwrap();

        let result = runtime.session_socket_path(OsStr::new(""));
        assert_eq!(result, Err(SessionNameError::Empty));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_session_socket_path_slash_rejected() {
        let dir = test_dir("sock_slash");
        create_safe_dir(&dir);
        let runtime = RuntimeDirectory::open(&dir).unwrap();

        let result = runtime.session_socket_path(OsStr::new("foo/bar"));
        assert_eq!(result, Err(SessionNameError::ContainsSlash));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_session_socket_path_nul_rejected() {
        let dir = test_dir("sock_nul");
        create_safe_dir(&dir);
        let runtime = RuntimeDirectory::open(&dir).unwrap();

        let name = OsStr::from_bytes(&[b'a', 0x00, b'b']);
        let result = runtime.session_socket_path(name);
        assert_eq!(result, Err(SessionNameError::ContainsNul));
        fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------
    // RuntimeDirectory::classify_session_socket
    // ------------------------------------------------------------------

    #[test]
    fn test_classify_session_socket_missing() {
        let dir = test_dir("cls_sock_missing");
        create_safe_dir(&dir);
        let runtime = RuntimeDirectory::open(&dir).unwrap();

        let status = runtime
            .classify_session_socket(OsStr::new("no-such-session"))
            .unwrap();
        assert_eq!(status, SocketPathStatus::Missing);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_classify_session_socket_invalid_name() {
        let dir = test_dir("cls_sock_invalid");
        create_safe_dir(&dir);
        let runtime = RuntimeDirectory::open(&dir).unwrap();

        let result = runtime.classify_session_socket(OsStr::new("foo/bar"));
        match result {
            Err(RuntimeDirectoryError::InvalidSessionName { name, source }) => {
                assert_eq!(name, OsString::from("foo/bar"));
                assert_eq!(source, SessionNameError::ContainsSlash);
            }
            other => panic!("expected InvalidSessionName, got: {:?}", other),
        }
        fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------
    // classify_socket_path (free function)
    // ------------------------------------------------------------------

    #[test]
    fn test_classify_missing_path() {
        let dir = test_dir("cls_missing");
        let result = classify_socket_path(dir.join("does-not-exist"));
        assert!(matches!(result, Ok(SocketPathStatus::Missing)));
    }

    #[test]
    fn test_classify_regular_file() {
        let dir = test_dir("cls_file");
        create_safe_dir(&dir);
        let path = dir.join("regular-file.txt");
        fs::write(&path, b"hello").unwrap();

        let result = classify_socket_path(&path);
        assert!(matches!(result, Ok(SocketPathStatus::RegularFile)));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_classify_directory_path() {
        let dir = test_dir("cls_dir");
        let subdir = dir.join("subdir");
        create_safe_dir(&subdir);

        let result = classify_socket_path(&subdir);
        assert!(matches!(result, Ok(SocketPathStatus::Directory)));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_classify_active_socket() {
        let dir = test_dir("cls_sock_active");
        create_safe_dir(&dir);
        let sock_path = dir.join("active.sock");

        // Bind a Unix listener — creates a socket inode.
        let listener =
            std::os::unix::net::UnixListener::bind(&sock_path).expect("bind test socket");

        let result = classify_socket_path(&sock_path).unwrap();
        assert_eq!(result, SocketPathStatus::ActiveSocket);

        // Drop the listener; the socket file remains on disk as a stale socket.
        drop(listener);
        fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------
    // Permission validation
    // ------------------------------------------------------------------

    #[test]
    fn test_permission_validation_rejects_group_writable() {
        let dir = test_dir("perm_group_wr");
        let runtime = RuntimeDirectory::create_private(&dir).unwrap();
        drop(runtime);

        // Change to group-writable (0o770)
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o770)).unwrap();

        let result = RuntimeDirectory::open(&dir);
        match result {
            Err(RuntimeDirectoryError::UnsafePermissions { path, mode }) => {
                assert_eq!(path, dir);
                assert!(
                    mode & 0o022 != 0,
                    "expected group/other write bits in {mode:o}"
                );
            }
            other => panic!("expected UnsafePermissions, got: {:?}", other),
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_permission_validation_accepts_safe_permissions() {
        let dir = test_dir("perm_safe");
        create_safe_dir(&dir);

        // 0o700 (rwx------) is completely safe
        let result = RuntimeDirectory::open(&dir);
        assert!(
            result.is_ok(),
            "safe perms should be accepted: {:?}",
            result.err()
        );

        // 0o755 (rwxr-xr-x) — group/other have r-x but not -w-, so it passes
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let result = RuntimeDirectory::open(&dir);
        assert!(
            result.is_ok(),
            "0o755 should be accepted: {:?}",
            result.err()
        );

        fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------------------
    // SessionNameError Display
    // ------------------------------------------------------------------

    #[test]
    fn test_session_name_validation_all_errors() {
        // Empty
        let result = validate_session_name(OsStr::new(""));
        assert_eq!(result, Err(SessionNameError::Empty));

        // Contains slash
        let result = validate_session_name(OsStr::new("a/b"));
        assert_eq!(result, Err(SessionNameError::ContainsSlash));

        // Contains NUL
        let result = validate_session_name(OsStr::from_bytes(&[b'a', 0x00, b'b']));
        assert_eq!(result, Err(SessionNameError::ContainsNul));

        // Valid
        assert!(validate_session_name(OsStr::new("valid-name")).is_ok());
        assert!(validate_session_name(OsStr::new("abc123._-")).is_ok());
    }

    #[test]
    fn test_session_name_error_display_messages() {
        assert_eq!(
            SessionNameError::Empty.to_string(),
            "empty names are not valid socket names"
        );
        assert_eq!(
            SessionNameError::ContainsSlash.to_string(),
            "slash is not valid in a socket name"
        );
        assert_eq!(
            SessionNameError::ContainsNul.to_string(),
            "nul is not valid in a socket name"
        );
    }

    // ------------------------------------------------------------------
    // RuntimeDirectoryError Display
    // ------------------------------------------------------------------

    #[test]
    fn test_runtime_directory_error_display() {
        let dir = PathBuf::from("/tmp/screen-test");
        let io_err = io::Error::new(io::ErrorKind::NotFound, "no such file");

        let err = RuntimeDirectoryError::Io {
            path: dir.clone(),
            source: io_err,
        };
        let msg = err.to_string();
        assert!(msg.contains("/tmp/screen-test"));

        let err = RuntimeDirectoryError::NotDirectory {
            path: dir.join("file"),
        };
        assert!(err.to_string().contains("is not a directory"));

        let err = RuntimeDirectoryError::WrongOwner {
            path: dir.clone(),
            expected_uid: 1000,
            actual_uid: 2000,
        };
        let msg = err.to_string();
        assert!(msg.contains("owned by uid 2000"));
        assert!(msg.contains("expected uid 1000"));

        let err = RuntimeDirectoryError::UnsafePermissions {
            path: dir.clone(),
            mode: 0o777,
        };
        let msg = err.to_string();
        assert!(msg.contains("777"));
        assert!(msg.contains("unsafe permissions"));

        let err = RuntimeDirectoryError::InvalidSessionName {
            name: OsString::from("bad"),
            source: SessionNameError::Empty,
        };
        let msg = err.to_string();
        assert!(msg.contains("invalid session name"));
    }

    // ------------------------------------------------------------------
    // Error trait source chain
    // ------------------------------------------------------------------

    #[test]
    fn test_error_source_chain() {
        use std::error::Error;

        // Io errors have a source
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
        let dir_err = RuntimeDirectoryError::Io {
            path: PathBuf::from("/tmp/x"),
            source: io_err,
        };
        assert!(dir_err.source().is_some());

        // InvalidSessionName has a source
        let name_err = RuntimeDirectoryError::InvalidSessionName {
            name: OsString::from(""),
            source: SessionNameError::Empty,
        };
        assert!(name_err.source().is_some());

        // Leaf errors have no source
        assert!(
            RuntimeDirectoryError::NotDirectory {
                path: PathBuf::from("/x")
            }
            .source()
            .is_none()
        );
        assert!(
            RuntimeDirectoryError::Symlink {
                path: PathBuf::from("/x")
            }
            .source()
            .is_none()
        );
        assert!(
            RuntimeDirectoryError::WrongOwner {
                path: PathBuf::from("/x"),
                expected_uid: 0,
                actual_uid: 0,
            }
            .source()
            .is_none()
        );
        assert!(
            RuntimeDirectoryError::UnsafePermissions {
                path: PathBuf::from("/x"),
                mode: 0,
            }
            .source()
            .is_none()
        );
    }
}
