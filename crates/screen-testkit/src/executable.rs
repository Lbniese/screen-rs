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
