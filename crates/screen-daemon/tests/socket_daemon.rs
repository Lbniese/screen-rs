use std::fs;
use std::io::{self, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use screen_daemon::{DaemonConfig, run_until_shutdown};
use screen_protocol::{Message, PROTOCOL_VERSION};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let path = PathBuf::from("/private/tmp").join(format!("sda-{name}-{nanos}"));
        fs::create_dir(&path).unwrap_or_else(|error| {
            panic!(
                "failed to create temporary directory {}: {error}",
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
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn daemon_accepts_hello_and_shutdown() {
    let temp = TempDir::new("shutdown");
    let socket_path = temp.path().join("daemon.sock");
    if !unix_socket_bind_allowed(&socket_path) {
        eprintln!("skipping daemon socket test: Unix socket bind is not permitted");
        return;
    }

    let daemon_path = socket_path.clone();
    let handle = thread::spawn(move || run_until_shutdown(DaemonConfig::new(daemon_path)));
    wait_for_path(&socket_path);

    let mut client = UnixStream::connect(&socket_path).expect("connect daemon");
    Message::Hello.write_to(&mut client).expect("write hello");
    assert_eq!(
        Message::read_from(&mut client).expect("read hello ack"),
        Message::HelloAck
    );

    Message::Shutdown
        .write_to(&mut client)
        .expect("write shutdown");
    assert_eq!(
        Message::read_from(&mut client).expect("read shutdown ack"),
        Message::ShutdownAck
    );

    let report = handle
        .join()
        .expect("daemon thread joins")
        .expect("daemon exits cleanly");
    assert_eq!(report.clients_served, 1);
    assert!(
        !socket_path.exists(),
        "daemon should remove socket on clean shutdown"
    );
}

#[test]
fn daemon_rejects_malformed_hello_without_shutting_down() {
    let temp = TempDir::new("malformed");
    let socket_path = temp.path().join("daemon.sock");
    if !unix_socket_bind_allowed(&socket_path) {
        eprintln!("skipping daemon socket test: Unix socket bind is not permitted");
        return;
    }

    let daemon_path = socket_path.clone();
    let handle = thread::spawn(move || run_until_shutdown(DaemonConfig::new(daemon_path)));
    wait_for_path(&socket_path);

    let mut malformed = UnixStream::connect(&socket_path).expect("connect daemon");
    malformed
        .write_all(&bad_magic_frame())
        .expect("write malformed frame");
    assert!(matches!(
        Message::read_from(&mut malformed),
        Ok(Message::Error(_))
    ));
    drop(malformed);

    let mut client = UnixStream::connect(&socket_path).expect("connect daemon after malformed");
    Message::Hello.write_to(&mut client).expect("write hello");
    assert_eq!(
        Message::read_from(&mut client).expect("read hello ack"),
        Message::HelloAck
    );
    Message::Shutdown
        .write_to(&mut client)
        .expect("write shutdown");
    assert_eq!(
        Message::read_from(&mut client).expect("read shutdown ack"),
        Message::ShutdownAck
    );

    let report = handle
        .join()
        .expect("daemon thread joins")
        .expect("daemon exits cleanly");
    assert_eq!(report.clients_served, 2);
}

fn unix_socket_bind_allowed(path: &Path) -> bool {
    match UnixListener::bind(path) {
        Ok(listener) => {
            drop(listener);
            let _ = fs::remove_file(path);
            true
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => false,
        Err(error) => panic!(
            "unexpected Unix socket bind error at {}: {error}",
            path.display()
        ),
    }
}

fn wait_for_path(path: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn bad_magic_frame() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BAD!");
    bytes.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes
}
