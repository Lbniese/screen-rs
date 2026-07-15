//! utmp/utmpx integration (stub).
//!
//! GNU Screen writes entries to the utmp database so that commands like
//! `who`, `w`, and `last` can see screen sessions.  Full utmp support
//! requires platform-specific libc bindings for `struct utmpx`,
//! `pututxline()`, `setutxent()`, and `endutxent()`.
//!
//! This module provides a best-effort stub — the API is defined but
//! actual utmp recording is deferred to a future release when all target
//! platform libc definitions are confirmed compatible with edition 2024
//! extern-block rules.

/// Write a login entry (best-effort, does nothing on current release).
pub fn write_login(line: &str, user: &str, host: &str) -> Result<(), UtmpError> {
    let _ = (line, user, host);
    // Deferred: full utmpx integration pending platform-specific libc audit.
    Ok(())
}

/// Write a logout entry (best-effort, does nothing on current release).
pub fn write_logout(line: &str) -> Result<(), UtmpError> {
    let _ = line;
    // Deferred: full utmpx integration pending platform-specific libc audit.
    Ok(())
}

#[derive(Debug)]
pub struct UtmpError {
    message: String,
}

impl std::fmt::Display for UtmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "utmp: {}", self.message)
    }
}

impl std::error::Error for UtmpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utmp_stub_does_not_panic() {
        assert!(write_login("screen-test", "nobody", "").is_ok());
        assert!(write_logout("screen-test").is_ok());
    }
}
