use std::ffi::{OsStr, OsString};
use std::process::ExitStatus;
use std::time::Duration;

use screen_pty::{PtyError, PtyProcess, PtySize};

#[derive(Debug)]
pub struct PtyTestProcess {
    process: PtyProcess,
}

impl PtyTestProcess {
    pub fn spawn<I, S>(program: impl AsRef<OsStr>, args: I, size: PtySize) -> Result<Self, PtyError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Ok(Self {
            process: PtyProcess::spawn(program, args, size)?,
        })
    }

    pub fn spawn_with_env<I, S, E, K, V>(
        program: impl AsRef<OsStr>,
        args: I,
        envs: E,
        size: PtySize,
    ) -> Result<Self, PtyError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        E: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let mut command = screen_pty::PtyCommand::new(program, size);
        command.args(args);
        for (key, value) in envs {
            command.env(key, value);
        }

        Ok(Self {
            process: command.spawn()?,
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.process.write_all(bytes)
    }

    pub fn read_until(&mut self, needle: &[u8], timeout: Duration) -> Result<Vec<u8>, PtyError> {
        self.process.read_until(needle, timeout)
    }

    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.process.resize(size)
    }

    pub fn wait_or_kill(&mut self, timeout: Duration) -> Result<ExitStatus, PtyError> {
        self.process.wait_or_kill(timeout)
    }
}
