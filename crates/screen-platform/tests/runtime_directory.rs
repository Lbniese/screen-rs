use std::fs::{self, File};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use screen_platform::{
    RuntimeDirectory, RuntimeDirectoryError, SocketPathStatus, current_effective_uid,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let short = if name.len() > 10 { &name[..10] } else { name };
        let path = PathBuf::from("/private/tmp").join(format!("sp-{short}-{nanos}-{id}"));
        fs::create_dir(&path).unwrap_or_else(|error| {
            panic!(
                "failed to create temporary directory {}: {error}",
                path.display()
            )
        });
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap_or_else(|error| {
            panic!(
                "failed to chmod temporary directory {}: {error}",
                path.display()
            )
        });
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o700));
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn safe_directory_is_accepted() {
    let temp = TempDir::new("safe-runtime");
    let runtime = RuntimeDirectory::open(temp.path()).expect("safe runtime directory");
    assert_eq!(runtime.path(), temp.path());
}

#[test]
fn world_writable_directory_is_rejected() {
    let temp = TempDir::new("world-writable-runtime");
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777))
        .expect("chmod should succeed");

    assert!(matches!(
        RuntimeDirectory::open(temp.path()),
        Err(RuntimeDirectoryError::UnsafePermissions { .. })
    ));
}

#[test]
fn wrong_owner_is_rejected_where_testable() {
    let temp = TempDir::new("wrong-owner-runtime");
    let wrong_uid = current_effective_uid().wrapping_add(1);

    assert!(matches!(
        RuntimeDirectory::open_for_owner(temp.path(), wrong_uid),
        Err(RuntimeDirectoryError::WrongOwner { .. })
    ));
}

#[test]
fn socket_status_distinguishes_active_and_stale_sockets() {
    let temp = TempDir::new("socket-status-runtime");
    let runtime = RuntimeDirectory::open(temp.path()).expect("safe runtime directory");
    let socket_path = runtime
        .session_socket_path("session".as_ref())
        .expect("valid socket name");
    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            eprintln!("skipping Unix socket bind test: {error}");
            return;
        }
        Err(error) => panic!("bind test socket: {error}"),
    };

    assert_eq!(
        runtime.classify_session_socket("session".as_ref()).unwrap(),
        SocketPathStatus::ActiveSocket
    );

    drop(listener);

    assert_eq!(
        runtime.classify_session_socket("session".as_ref()).unwrap(),
        SocketPathStatus::StaleSocket
    );
}

#[test]
fn regular_file_at_socket_path_is_rejected_by_classification() {
    let temp = TempDir::new("regular-file-runtime");
    let runtime = RuntimeDirectory::open(temp.path()).expect("safe runtime directory");
    let socket_path = runtime
        .session_socket_path("session".as_ref())
        .expect("valid socket name");
    File::create(socket_path).expect("create regular file");

    assert_eq!(
        runtime.classify_session_socket("session".as_ref()).unwrap(),
        SocketPathStatus::RegularFile
    );
}

#[test]
fn session_names_cannot_escape_runtime_directory() {
    let temp = TempDir::new("session-name-runtime");
    let runtime = RuntimeDirectory::open(temp.path()).expect("safe runtime directory");

    assert!(runtime.session_socket_path("../outside".as_ref()).is_err());
    assert!(runtime.session_socket_path("".as_ref()).is_err());
}
