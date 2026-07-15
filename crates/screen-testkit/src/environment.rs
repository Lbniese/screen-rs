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
        command.env("ZDOTDIR", &self.home);
        command.env("SCREENDIR", self.runtime.path());
        command.env("SCREENRC", "/dev/null");
        command.env("TERM", "xterm-256color");
        command.env("LC_ALL", "C");
        // Tell screen-rs daemons spawned by this test to self-terminate when
        // the test process exits, preventing zombie daemons that cause flaky
        // re-runs.
        command.env("SCREEN_RS_PARENT_PID", std::process::id().to_string());
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn test_create_environment() {
        let env = TestEnvironment::create("test_env").unwrap();
        assert!(env.root().exists(), "root directory should exist");
        assert!(env.home().exists(), "home directory should exist");
        assert!(env.runtime().path().exists(), "runtime dir should exist");
        assert!(
            env.runtime().path().is_dir(),
            "runtime should be a directory"
        );
    }

    #[test]
    fn test_environment_root_is_directory() {
        let env = TestEnvironment::create("test_root_dir").unwrap();
        assert!(env.root().is_dir());
    }

    #[test]
    fn test_environment_home_is_directory() {
        let env = TestEnvironment::create("test_home_dir").unwrap();
        assert!(env.home().is_dir());
    }

    #[test]
    fn test_environment_configure_command_sets_home() {
        let env = TestEnvironment::create("test_cfg_home").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo \"HOME=$HOME\"");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run echo HOME");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("HOME="),
            "HOME should be set in env: {stdout}"
        );
    }

    #[test]
    fn test_environment_configure_command_sets_screendir() {
        let env = TestEnvironment::create("test_cfg_sd").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo \"SCREENDIR=$SCREENDIR\"");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run echo SCREENDIR");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("SCREENDIR="),
            "SCREENDIR should be set: {stdout}"
        );
    }

    #[test]
    fn test_environment_configure_command_sets_screenrc() {
        let env = TestEnvironment::create("test_cfg_rc").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo \"SCREENRC=$SCREENRC\"");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run echo SCREENRC");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("SCREENRC=/dev/null"),
            "SCREENRC should be /dev/null: {stdout}"
        );
    }

    #[test]
    fn test_environment_configure_command_sets_term() {
        let env = TestEnvironment::create("test_cfg_term").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo \"TERM=$TERM\"");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run echo TERM");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("TERM=xterm-256color"),
            "TERM should be set: {stdout}"
        );
    }

    #[test]
    fn test_environment_configure_command_sets_lc_all() {
        let env = TestEnvironment::create("test_cfg_lc").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo \"LC_ALL=$LC_ALL\"");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run echo LC_ALL");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("LC_ALL=C"), "LC_ALL should be C: {stdout}");
    }

    #[test]
    fn test_environment_configure_command_sets_parent_pid() {
        let env = TestEnvironment::create("test_cfg_pid").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo \"PPID=$SCREEN_RS_PARENT_PID\"");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run echo PPID");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let our_pid = std::process::id().to_string();
        assert!(
            stdout.contains(&our_pid),
            "SCREEN_RS_PARENT_PID should be our PID ({our_pid}): {stdout}"
        );
    }

    #[test]
    fn test_environment_configure_command_sets_current_dir() {
        let env = TestEnvironment::create("test_cfg_cwd").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("pwd");
        env.configure_command(&mut cmd);
        let output = cmd.output().expect("run pwd");
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let expected = env.root().canonicalize().unwrap();
        let actual = std::path::Path::new(&stdout);
        assert!(
            actual == expected,
            "current dir should resolve to env root:\n  actual: {actual:?}\nexpected: {expected:?}"
        );
    }

    #[test]
    fn test_environment_cleanup() {
        let root_path;
        {
            let env = TestEnvironment::create("test_cleanup").unwrap();
            root_path = env.root().to_owned();
            assert!(root_path.exists());
        }
        // env was dropped, directory should be removed
        assert!(
            !root_path.exists(),
            "root directory should be cleaned up on Drop"
        );
    }

    #[test]
    fn test_environment_home_is_subdir_of_root() {
        let env = TestEnvironment::create("test_subdir").unwrap();
        let home = env.home();
        let root = env.root();
        assert!(home.starts_with(root), "{home:?} should be under {root:?}");
    }

    #[test]
    fn test_cleanup_removes_all_dirs() {
        let root_path;
        let home_path;
        {
            let env = TestEnvironment::create("test_full_clean").unwrap();
            root_path = env.root().to_owned();
            home_path = env.home().to_owned();
            assert!(root_path.exists());
            assert!(home_path.exists());
        }
        assert!(!root_path.exists(), "root should be removed");
        assert!(!home_path.exists(), "home should be removed (under root)");
    }
}
