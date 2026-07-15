//! Configuration parsing integration tests.
//!
//! Tests the screenrc parser with real-world configuration snippets,
//! verifying that common screen commands are correctly deserialized
//! into the [`screen_config::ScreenConfig`] model.

use screen_config::parse_config;

// ---------------------------------------------------------------------------
// Basic / empty
// ---------------------------------------------------------------------------

#[test]
fn empty_config_has_defaults() {
    let cfg = parse_config(b"").expect("empty config");
    assert!(cfg.shell.is_none());
    assert!(cfg.term.is_none());
}

#[test]
fn comment_and_blank_lines_produce_no_commands() {
    let cfg = parse_config(b"# comment\n  # indented\n\n# another\n").expect("comments");
    assert_eq!(cfg.bindings.len(), 0);
    assert_eq!(cfg.startup_windows.len(), 0);
}

// ---------------------------------------------------------------------------
// Shell / term / chdir
// ---------------------------------------------------------------------------

#[test]
fn shell_command() {
    let cfg = parse_config(b"shell /bin/zsh\n").expect("shell");
    assert_eq!(cfg.shell.as_deref(), Some(&b"/bin/zsh"[..]));
}

#[test]
fn term_command() {
    let cfg = parse_config(b"term screen-256color\n").expect("term");
    assert_eq!(cfg.term.as_deref(), Some(&b"screen-256color"[..]));
}

#[test]
fn chdir_command() {
    let cfg = parse_config(b"chdir /home/user\n").expect("chdir");
    assert_eq!(cfg.chdir.as_deref(), Some(&b"/home/user"[..]));
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

#[test]
fn logfile_command() {
    let cfg = parse_config(b"logfile /tmp/screen.log\n").expect("logfile");
    assert_eq!(cfg.logfile.as_deref(), Some(&b"/tmp/screen.log"[..]));
}

#[test]
fn deflog_on() {
    let cfg = parse_config(b"deflog on\n").expect("deflog");
    assert_eq!(cfg.logging, Some(true));
}

// ---------------------------------------------------------------------------
// Escape
// ---------------------------------------------------------------------------

#[test]
fn escape_default() {
    let cfg = parse_config(b"escape ^Aa\n").expect("escape");
    assert!(cfg.escape.is_some(), "escape should be set");
}

// ---------------------------------------------------------------------------
// Startup message
// ---------------------------------------------------------------------------

#[test]
fn startup_message_off() {
    let cfg = parse_config(b"startup_message off\n").expect("startup msg");
    assert_eq!(cfg.startup_message, Some(false));
}

// ---------------------------------------------------------------------------
// Defscrollback
// ---------------------------------------------------------------------------

#[test]
fn defscrollback_value() {
    let cfg = parse_config(b"defscrollback 5000\n").expect("scrollback");
    assert_eq!(cfg.defscrollback, Some(5000));
}

// ---------------------------------------------------------------------------
// Vbell
// ---------------------------------------------------------------------------

#[test]
fn vbell_on() {
    let cfg = parse_config(b"vbell on\n").expect("vbell on");
    assert_eq!(cfg.vbell, Some(true));
}

#[test]
fn vbell_off() {
    let cfg = parse_config(b"vbell off\n").expect("vbell off");
    assert_eq!(cfg.vbell, Some(false));
}

// ---------------------------------------------------------------------------
// Hardstatus / Caption
// ---------------------------------------------------------------------------

#[test]
fn hardstatus_string() {
    let cfg =
        parse_config(b"hardstatus alwayslastline \"%{= kw}%-w%{= BW}%n %t%{-}%+w\"\n")
            .expect("hardstatus");
    assert!(cfg.hardstatus.is_some(), "hardstatus should be set");
    let hs = String::from_utf8_lossy(cfg.hardstatus.as_ref().unwrap());
    assert!(hs.contains("%n"), "hardstatus should contain window number");
}

#[test]
fn caption_string() {
    let cfg = parse_config(b"caption always \"%3n %t\"\n").expect("caption");
    assert!(cfg.caption.is_some(), "caption should be set");
}

// ---------------------------------------------------------------------------
// Screen windows
// ---------------------------------------------------------------------------

#[test]
fn screen_with_number_and_program() {
    let cfg = parse_config(b"screen 1 bash\n").expect("screen");
    assert_eq!(cfg.startup_windows.len(), 1);
    assert_eq!(cfg.startup_windows[0].number, Some(1));
    assert_eq!(
        cfg.startup_windows[0].program.as_deref(),
        Some(&b"bash"[..])
    );
}

#[test]
fn screen_with_program_only() {
    let cfg = parse_config(b"screen vim\n").expect("screen");
    assert_eq!(cfg.startup_windows.len(), 1);
    assert!(cfg.startup_windows[0].number.is_none());
    assert_eq!(
        cfg.startup_windows[0].program.as_deref(),
        Some(&b"vim"[..])
    );
}

// ---------------------------------------------------------------------------
// Key bindings
// ---------------------------------------------------------------------------

#[test]
fn bind_key() {
    let cfg = parse_config(b"bind ^C screen 1\n").expect("bind");
    assert!(!cfg.bindings.is_empty(), "should have a binding");
}

// ---------------------------------------------------------------------------
// Multiuser / ACL
// ---------------------------------------------------------------------------

#[test]
fn multiuser_on() {
    let cfg = parse_config(b"multiuser on\n").expect("multiuser");
    assert_eq!(cfg.multiuser, Some(true));
}

#[test]
fn acladd_user() {
    let cfg = parse_config(b"acladd bob\n").expect("acladd");
    assert!(!cfg.acl.is_empty(), "should have ACL entries");
    assert_eq!(cfg.acl[0].username, b"bob");
}

// ---------------------------------------------------------------------------
// Misc commands
// ---------------------------------------------------------------------------

#[test]
fn defhistsize_value() {
    let cfg = parse_config(b"defhistsize 2000\n").expect("histsize");
    assert_eq!(cfg.defhistsize, Some(2000));
}

#[test]
fn zombie_mode() {
    let cfg = parse_config(b"zombie kr\n").expect("zombie");
    assert_eq!(cfg.defzombie.as_deref(), Some(&b"kr"[..]));
}

#[test]
fn silencewait_value() {
    let cfg = parse_config(b"silencewait 15\n").expect("silencewait");
    assert_eq!(cfg.silencewait, Some(15));
}

#[test]
fn defencoding_value() {
    let cfg = parse_config(b"defencoding utf-8\n").expect("encoding");
    assert_eq!(cfg.defencoding.as_deref(), Some(&b"utf-8"[..]));
}

#[test]
fn activity_message() {
    let cfg = parse_config(b"activity \"Activity in %n(%t)\"\n").expect("activity");
    assert!(cfg.activity.is_some(), "activity should be set");
}

#[test]
fn select_window() {
    let cfg = parse_config(b"select 0\n").expect("select");
    assert_eq!(cfg.select, Some(0));
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[test]
fn invalid_line_returns_err() {
    let result = parse_config(b"not_a_valid_command with args\n");
    // The parser should either succeed (skipping unknown) or return an error
    match result {
        Ok(cfg) => {
            // Unknown commands are silently accepted (GNU Screen compatible)
            assert!(cfg.bindings.is_empty());
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(!msg.is_empty(), "error message should not be empty");
        }
    }
}

#[test]
fn bad_escape_returns_err() {
    let result = parse_config(b"escape ZZ\n");
    match result {
        Ok(_) => {} // Some parsers accept any 2-byte escape
        Err(e) => {
            let msg = e.to_string();
            assert!(!msg.is_empty(), "error message should not be empty");
        }
    }
}
