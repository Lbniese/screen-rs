use std::error::Error;
use std::ffi::{OsStr, OsString, c_int, c_ulong, c_void};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(not(unix))]
compile_error!("screen-pty currently supports Unix targets only");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub columns: u16,
    pub rows: u16,
}

impl PtySize {
    pub const fn new(columns: u16, rows: u16) -> Self {
        Self { columns, rows }
    }
}

#[derive(Debug)]
pub struct PtyProcess {
    master: File,
    child: Child,
}

impl PtyProcess {
    pub fn spawn<I, S>(program: impl AsRef<OsStr>, args: I, size: PtySize) -> Result<Self, PtyError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = PtyCommand::new(program, size);
        command.args(args);
        command.spawn()
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.master.write_all(bytes).map_err(PtyError::Write)
    }

    pub fn read_available(&mut self) -> Result<Vec<u8>, PtyError> {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];

        loop {
            match self.master.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => output.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // Linux returns EIO (5) when the PTY slave is closed (child exited).
                // Treat it like EOF rather than a fatal error so the daemon can
                // detect the child exit via try_wait and send ChildExited.
                Err(error) if error.raw_os_error() == Some(5) => break,
                Err(error) => return Err(PtyError::Read(error)),
            }
        }

        Ok(output)
    }

    pub fn read_until(&mut self, needle: &[u8], timeout: Duration) -> Result<Vec<u8>, PtyError> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();

        loop {
            output.extend(self.read_available()?);
            if contains_bytes(&output, needle) {
                return Ok(output);
            }
            if Instant::now() >= deadline {
                return Err(PtyError::Timeout {
                    operation: "read_until",
                    timeout,
                    output,
                });
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        set_window_size(self.master.as_raw_fd(), size)
    }

    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ExitStatus>, PtyError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().map_err(PtyError::Wait)? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn wait_or_kill(&mut self, timeout: Duration) -> Result<ExitStatus, PtyError> {
        if let Some(status) = self.wait_timeout(timeout)? {
            return Ok(status);
        }

        terminate_child_group(&mut self.child)?;
        if let Some(status) = self.wait_timeout(Duration::from_secs(2))? {
            return Ok(status);
        }

        Err(PtyError::Timeout {
            operation: "wait_or_kill",
            timeout: timeout + Duration::from_secs(2),
            output: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct PtyCommand {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    current_dir: Option<PathBuf>,
    size: PtySize,
}

impl PtyCommand {
    pub fn new(program: impl AsRef<OsStr>, size: PtySize) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            envs: Vec::new(),
            current_dir: None,
            size,
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    pub fn env(&mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> &mut Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    pub fn current_dir(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.current_dir = Some(path.into());
        self
    }

    pub fn spawn(&self) -> Result<PtyProcess, PtyError> {
        let pair = PtyPair::open(self.size)?;
        let master_fd = pair.master.as_raw_fd();
        let slave_fd = pair.slave.as_raw_fd();
        set_close_on_exec(master_fd)?;
        set_close_on_exec(slave_fd)?;
        set_nonblocking(master_fd)?;

        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(current_dir) = &self.current_dir {
            command.current_dir(current_dir);
        }
        for (key, value) in &self.envs {
            command.env(key, value);
        }
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());

        // SAFETY: The pre-exec closure only calls async-signal-safe POSIX functions
        // needed to make the PTY slave the child's controlling terminal and stdio.
        // It captures only the integer slave fd, whose lifetime is valid until spawn
        // finishes because `pair.slave` is still owned in this function.
        unsafe {
            command.pre_exec(move || configure_child_pty(slave_fd));
        }

        let child = command.spawn().map_err(PtyError::Spawn)?;
        drop(pair.slave);

        Ok(PtyProcess {
            master: pair.master,
            child,
        })
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = kill_process_group(self.child.id());
            let _ = self.child.kill();
            let _ = wait_for_child_exit(&mut self.child, Duration::from_secs(1));
        }
    }
}

#[derive(Debug)]
pub enum PtyError {
    Open(io::Error),
    Fcntl(io::Error),
    Spawn(io::Error),
    Read(io::Error),
    Write(io::Error),
    Resize(io::Error),
    Wait(io::Error),
    Kill(io::Error),
    Timeout {
        operation: &'static str,
        timeout: Duration,
        output: Vec<u8>,
    },
}

impl fmt::Display for PtyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(error) => write!(formatter, "failed to open PTY: {error}"),
            Self::Fcntl(error) => write!(formatter, "failed to update PTY fd flags: {error}"),
            Self::Spawn(error) => write!(formatter, "failed to spawn PTY child: {error}"),
            Self::Read(error) => write!(formatter, "failed to read PTY output: {error}"),
            Self::Write(error) => write!(formatter, "failed to write PTY input: {error}"),
            Self::Resize(error) => write!(formatter, "failed to resize PTY: {error}"),
            Self::Wait(error) => write!(formatter, "failed to wait for PTY child: {error}"),
            Self::Kill(error) => write!(formatter, "failed to kill PTY child: {error}"),
            Self::Timeout {
                operation, timeout, ..
            } => write!(
                formatter,
                "PTY operation {operation} timed out after {timeout:?}"
            ),
        }
    }
}

impl Error for PtyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open(error)
            | Self::Fcntl(error)
            | Self::Spawn(error)
            | Self::Read(error)
            | Self::Write(error)
            | Self::Resize(error)
            | Self::Wait(error)
            | Self::Kill(error) => Some(error),
            Self::Timeout { .. } => None,
        }
    }
}

#[derive(Debug)]
struct PtyPair {
    master: File,
    slave: File,
}

impl PtyPair {
    fn open(size: PtySize) -> Result<Self, PtyError> {
        let mut master_fd = -1;
        let mut slave_fd = -1;
        let winsize = Winsize::from(size);

        // SAFETY: `openpty` writes two owned file descriptors into valid pointers.
        // The termios pointer and name pointer are null because this layer does not
        // customize terminal attributes or need the slave path. On success both fds
        // are immediately wrapped in `File` so RAII owns cleanup.
        let result = unsafe {
            openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &winsize,
            )
        };

        if result == -1 {
            return Err(PtyError::Open(io::Error::last_os_error()));
        }

        // SAFETY: `openpty` returned a valid owned master fd and ownership is moved
        // into `File`; no other Rust value will close it.
        let master = unsafe { File::from_raw_fd(master_fd) };
        // SAFETY: `openpty` returned a valid owned slave fd and ownership is moved
        // into `File`; no other Rust value will close it.
        let slave = unsafe { File::from_raw_fd(slave_fd) };

        Ok(Self { master, slave })
    }
}

/// Quick probe: try to open a PTY pair and immediately drop it.
/// Returns true if PTY allocation is available on this system.
pub fn pty_available() -> bool {
    PtyPair::open(PtySize::new(80, 24)).is_ok()
}

fn configure_child_pty(slave_fd: RawFd) -> io::Result<()> {
    // SAFETY: `setsid` has no pointer arguments and is called in the child just
    // before exec to create a new session for the PTY-controlled process.
    if unsafe { setsid() } == -1 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `slave_fd` is an open PTY slave fd inherited by the child. TIOCSCTTY
    // makes it the controlling terminal for the new session.
    if unsafe { ioctl(slave_fd, TIOCSCTTY, 0 as c_int) } == -1 {
        return Err(io::Error::last_os_error());
    }

    duplicate_to_stdio(slave_fd)
}

fn duplicate_to_stdio(slave_fd: RawFd) -> io::Result<()> {
    for target in [STDIN_FILENO, STDOUT_FILENO, STDERR_FILENO] {
        // SAFETY: `slave_fd` is valid in the child, and `target` is one of the
        // standard descriptor numbers. `dup2` atomically replaces that descriptor.
        if unsafe { dup2(slave_fd, target) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn set_close_on_exec(fd: RawFd) -> Result<(), PtyError> {
    // SAFETY: `fd` is an open file descriptor owned by this process. `fcntl` with
    // F_GETFD does not take pointer arguments and returns descriptor flags.
    let flags = unsafe { fcntl(fd, F_GETFD) };
    if flags == -1 {
        return Err(PtyError::Fcntl(io::Error::last_os_error()));
    }
    // SAFETY: `fd` remains valid, and F_SETFD stores the updated close-on-exec flag.
    if unsafe { fcntl(fd, F_SETFD, flags | FD_CLOEXEC) } == -1 {
        return Err(PtyError::Fcntl(io::Error::last_os_error()));
    }
    Ok(())
}

fn set_nonblocking(fd: RawFd) -> Result<(), PtyError> {
    // SAFETY: `fd` is an open file descriptor owned by this process. `fcntl` with
    // F_GETFL does not take pointer arguments and returns status flags.
    let flags = unsafe { fcntl(fd, F_GETFL) };
    if flags == -1 {
        return Err(PtyError::Fcntl(io::Error::last_os_error()));
    }
    // SAFETY: `fd` remains valid, and F_SETFL stores the updated nonblocking flag.
    if unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) } == -1 {
        return Err(PtyError::Fcntl(io::Error::last_os_error()));
    }
    Ok(())
}

fn set_window_size(fd: RawFd, size: PtySize) -> Result<(), PtyError> {
    let winsize = Winsize::from(size);
    // SAFETY: `fd` is an open PTY master fd, and `winsize` points to a valid
    // `struct winsize` for the duration of the ioctl call.
    if unsafe { ioctl(fd, TIOCSWINSZ, &winsize) } == -1 {
        return Err(PtyError::Resize(io::Error::last_os_error()));
    }
    Ok(())
}

fn terminate_child_group(child: &mut Child) -> Result<(), PtyError> {
    let group_result = kill_process_group(child.id());
    let child_result = child.kill();

    match (group_result, child_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(group_error), Err(child_error)) => {
            if is_process_gone(&group_error) || is_process_gone(&child_error) {
                Ok(())
            } else {
                Err(PtyError::Kill(child_error))
            }
        }
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn kill_process_group(process_id: u32) -> io::Result<()> {
    let process_id = process_id as c_int;
    let result = unsafe { kill(-process_id, SIGKILL) };
    if result == -1 {
        let error = io::Error::last_os_error();
        if is_process_gone(&error) {
            Ok(())
        } else {
            Err(error)
        }
    } else {
        Ok(())
    }
}

fn is_process_gone(error: &io::Error) -> bool {
    error.raw_os_error() == Some(ESRCH)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

impl From<PtySize> for Winsize {
    fn from(size: PtySize) -> Self {
        Self {
            ws_row: size.rows,
            ws_col: size.columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}

const STDIN_FILENO: c_int = 0;
const STDOUT_FILENO: c_int = 1;
const STDERR_FILENO: c_int = 2;
const SIGKILL: c_int = 9;
const ESRCH: c_int = 3;
const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const FD_CLOEXEC: c_int = 1;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_NONBLOCK: c_int = 0x0004;
#[cfg(target_os = "linux")]
const O_NONBLOCK: c_int = 0o4000;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const TIOCSCTTY: c_ulong = 0x2000_7461;
#[cfg(target_os = "linux")]
const TIOCSCTTY: c_ulong = 0x540E;

#[cfg(any(target_os = "macos", target_os = "ios"))]
const TIOCSWINSZ: c_ulong = 0x8008_7467;
#[cfg(target_os = "linux")]
const TIOCSWINSZ: c_ulong = 0x5414;

#[cfg_attr(target_os = "linux", link(name = "util"))]
unsafe extern "C" {
    fn openpty(
        amaster: *mut c_int,
        aslave: *mut c_int,
        name: *mut std::ffi::c_char,
        termp: *const c_void,
        winp: *const Winsize,
    ) -> c_int;
    fn setsid() -> c_int;
    fn dup2(old_fd: c_int, new_fd: c_int) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
}
