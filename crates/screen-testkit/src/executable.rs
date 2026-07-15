use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTimeError};

use screen_platform::RuntimeDirectoryError;

use crate::TestEnvironment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenExecutable {
    pub path: PathBuf,
}

impl ScreenExecutable {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn run<I, S>(
        &self,
        args: I,
        environment: &TestEnvironment,
    ) -> Result<CommandResult, TestError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with_timeout(args, environment, Duration::from_secs(5))
    }

    pub fn run_with_timeout<I, S>(
        &self,
        args: I,
        environment: &TestEnvironment,
        timeout: Duration,
    ) -> Result<CommandResult, TestError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect();
        let mut command = Command::new(&self.path);
        command.args(&args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        environment.configure_command(&mut command);

        let mut child = command.spawn().map_err(|source| TestError::Spawn {
            path: self.path.clone(),
            source,
        })?;

        let stdout = child.stdout.take().ok_or(TestError::MissingPipe {
            stream: "stdout",
            path: self.path.clone(),
        })?;
        let stderr = child.stderr.take().ok_or(TestError::MissingPipe {
            stream: "stderr",
            path: self.path.clone(),
        })?;

        let stdout_thread = thread::spawn(move || read_all(stdout));
        let stderr_thread = thread::spawn(move || read_all(stderr));

        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|source| TestError::Wait {
                path: self.path.clone(),
                source,
            })? {
                break status;
            }

            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_reader(stdout_thread, "stdout", &self.path)?;
                let stderr = join_reader(stderr_thread, "stderr", &self.path)?;
                return Err(TestError::Timeout {
                    path: self.path.clone(),
                    args,
                    timeout,
                    stdout,
                    stderr,
                });
            }

            thread::sleep(Duration::from_millis(10));
        };

        let stdout = join_reader(stdout_thread, "stdout", &self.path)?;
        let stderr = join_reader(stderr_thread, "stderr", &self.path)?;

        Ok(CommandResult {
            status,
            stdout,
            stderr,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum TestError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Runtime(RuntimeDirectoryError),
    Clock(SystemTimeError),
    Spawn {
        path: PathBuf,
        source: io::Error,
    },
    Wait {
        path: PathBuf,
        source: io::Error,
    },
    MissingPipe {
        stream: &'static str,
        path: PathBuf,
    },
    Read {
        stream: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    ReaderPanicked {
        stream: &'static str,
        path: PathBuf,
    },
    Timeout {
        path: PathBuf,
        args: Vec<OsString>,
        timeout: Duration,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Runtime(error) => write!(formatter, "{error}"),
            Self::Clock(error) => write!(formatter, "system clock error: {error}"),
            Self::Spawn { path, source } => {
                write!(formatter, "failed to spawn {}: {source}", path.display())
            }
            Self::Wait { path, source } => {
                write!(formatter, "failed to wait for {}: {source}", path.display())
            }
            Self::MissingPipe { stream, path } => {
                write!(formatter, "missing {stream} pipe for {}", path.display())
            }
            Self::Read {
                stream,
                path,
                source,
            } => write!(
                formatter,
                "failed to read {stream} for {}: {source}",
                path.display()
            ),
            Self::ReaderPanicked { stream, path } => {
                write!(formatter, "{stream} reader panicked for {}", path.display())
            }
            Self::Timeout {
                path,
                args,
                timeout,
                ..
            } => write!(
                formatter,
                "{} {:?} timed out after {:?}",
                path.display(),
                args,
                timeout
            ),
        }
    }
}

impl Error for TestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Runtime(error) => Some(error),
            Self::Clock(error) => Some(error),
            Self::Spawn { source, .. } => Some(source),
            Self::Wait { source, .. } => Some(source),
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn default_reference_path() -> PathBuf {
    std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"))
}

fn read_all(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    Ok(output)
}

fn join_reader(
    handle: thread::JoinHandle<io::Result<Vec<u8>>>,
    stream: &'static str,
    path: &std::path::Path,
) -> Result<Vec<u8>, TestError> {
    match handle.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(source)) => Err(TestError::Read {
            stream,
            path: path.to_owned(),
            source,
        }),
        Err(_panic) => Err(TestError::ReaderPanicked {
            stream,
            path: path.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::time::SystemTime;

    #[test]
    fn test_create_executable() {
        let exe = ScreenExecutable::new("/usr/bin/true");
        assert_eq!(exe.path, PathBuf::from("/usr/bin/true"));
    }

    #[test]
    fn test_create_executable_from_string() {
        let exe = ScreenExecutable::new("/bin/sh");
        assert_eq!(exe.path, PathBuf::from("/bin/sh"));
    }

    #[test]
    fn test_default_reference_path_defaults() {
        // Without SCREEN_REFERENCE env, should default to "screen"
        let path = default_reference_path();
        assert_eq!(path, PathBuf::from("screen"));
    }

    #[test]
    fn test_test_error_display_io() {
        let err = TestError::Io {
            path: PathBuf::from("/fake/path"),
            source: io::Error::new(io::ErrorKind::NotFound, "file not found"),
        };
        let msg = err.to_string();
        assert!(msg.contains("/fake/path"), "msg={msg}");
        assert!(msg.contains("file not found"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_spawn() {
        let err = TestError::Spawn {
            path: PathBuf::from("/bin/fake"),
            source: io::Error::new(io::ErrorKind::NotFound, "no such file"),
        };
        let msg = err.to_string();
        assert!(msg.contains("/bin/fake"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_wait() {
        let err = TestError::Wait {
            path: PathBuf::from("/bin/sleep"),
            source: io::Error::other("interrupted"),
        };
        let msg = err.to_string();
        assert!(msg.contains("/bin/sleep"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_missing_pipe() {
        let err = TestError::MissingPipe {
            stream: "stdout",
            path: PathBuf::from("/bin/test"),
        };
        let msg = err.to_string();
        assert!(msg.contains("stdout"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_read() {
        let err = TestError::Read {
            stream: "stderr",
            path: PathBuf::from("/bin/test"),
            source: io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"),
        };
        let msg = err.to_string();
        assert!(msg.contains("stderr"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_reader_panicked() {
        let err = TestError::ReaderPanicked {
            stream: "stdout",
            path: PathBuf::from("/bin/test"),
        };
        let msg = err.to_string();
        assert!(msg.contains("stdout"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_timeout() {
        let err = TestError::Timeout {
            path: PathBuf::from("/bin/sleep"),
            args: vec![OsString::from("100")],
            timeout: Duration::from_secs(1),
            stdout: vec![],
            stderr: vec![],
        };
        let msg = err.to_string();
        assert!(msg.contains("timed out"), "msg={msg}");
    }

    #[test]
    fn test_test_error_display_runtime() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let dir_err = screen_platform::RuntimeDirectoryError::Io {
            path: PathBuf::from("/run/screen"),
            source: io_err,
        };
        let err = TestError::Runtime(dir_err);
        let msg = err.to_string();
        assert!(
            !msg.is_empty(),
            "Runtime error Display should produce output"
        );
    }

    #[test]
    fn test_test_error_display_clock() {
        let err = TestError::Clock(
            SystemTime::UNIX_EPOCH
                .duration_since(SystemTime::now())
                .unwrap_err(),
        );
        let msg = err.to_string();
        assert!(msg.contains("clock"), "msg={msg}");
    }

    #[test]
    fn test_test_error_source_io() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let err = TestError::Io {
            path: PathBuf::from("/x"),
            source: io_err,
        };
        assert!(err.source().is_some());
    }

    #[test]
    fn test_test_error_source_spawn() {
        let err = TestError::Spawn {
            path: PathBuf::from("/x"),
            source: io::Error::new(io::ErrorKind::NotFound, "x"),
        };
        assert!(err.source().is_some());
    }

    #[test]
    fn test_test_error_source_timeout() {
        let err = TestError::Timeout {
            path: PathBuf::from("/x"),
            args: vec![],
            timeout: Duration::from_secs(1),
            stdout: vec![],
            stderr: vec![],
        };
        assert!(err.source().is_none());
    }

    #[test]
    fn test_test_error_source_missing_pipe() {
        let err = TestError::MissingPipe {
            stream: "out",
            path: PathBuf::from("/x"),
        };
        assert!(err.source().is_none());
    }

    #[test]
    fn test_command_result_debug() {
        let result = CommandResult {
            status: ExitStatus::from_raw(0),
            stdout: b"hello".to_vec(),
            stderr: b"".to_vec(),
        };
        let debug = format!("{result:?}");
        // Debug output shows "ExitStatus(unix_wait_status(0))" and bytes as integers
        assert!(debug.contains("CommandResult"), "debug={debug}");
        assert!(debug.contains("ExitStatus"), "debug={debug}");
        // bytes 104='h', 101='e', 108='l', 108='l', 111='o'
        assert!(debug.contains("104"), "debug={debug}");
        assert!(debug.contains("111"), "debug={debug}");
    }
}
