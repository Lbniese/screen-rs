use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use screen_platform::RuntimeDirectory;

use crate::TestError;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct TestEnvironment {
    root: PathBuf,
    home: PathBuf,
    runtime: RuntimeDirectory,
}

impl TestEnvironment {
    pub fn create(prefix: &str) -> Result<Self, TestError> {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(TestError::Clock)?
            .subsec_nanos();
        // Keep path short: temp dir / prefix-nanos-id / home|screen
        let short = if prefix.len() > 10 {
            &prefix[..10]
        } else {
            prefix
        };
        let root = std::env::temp_dir().join(format!("st-{short}-{nanos}-{id}"));
        let home = root.join("h");
        let runtime_path = root.join("s");

        fs::create_dir(&root).map_err(|source| TestError::Io {
            path: root.clone(),
            source,
        })?;
        fs::create_dir(&home).map_err(|source| TestError::Io {
            path: home.clone(),
            source,
        })?;
        let runtime =
            RuntimeDirectory::create_private(&runtime_path).map_err(TestError::Runtime)?;

        Ok(Self {
            root,
            home,
            runtime,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn runtime(&self) -> &RuntimeDirectory {
        &self.runtime
    }

    pub fn configure_command(&self, command: &mut Command) {
        command.current_dir(&self.root);
        command.env("HOME", &self.home);
        command.env("SCREENDIR", self.runtime.path());
        command.env("SCREENRC", "/dev/null");
        command.env("TERM", "xterm-256color");
        command.env("LC_ALL", "C");
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
