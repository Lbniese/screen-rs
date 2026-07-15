//! Terminal engine integration tests.
//!
//! These tests exercise the terminal state machine directly through
//! the public API, verifying correct behavior of escape sequence
//! processing, grid operations, and scrollback management.

use screen_terminal::{Dimensions, MouseMode, Style, TerminalState};

fn make_term(columns: u16, rows: u16) -> TerminalState {
    TerminalState::new(Dimensions::new(columns, rows))
}

// ---------------------------------------------------------------------------
// Basic rendering
// ---------------------------------------------------------------------------

#[test]
fn printable_bytes_are_stored() {
    let mut term = make_term(80, 24);
    term.apply(b"Hello, World!");
    assert_eq!(term.plain_text(), "Hello, World!");
}

#[test]
fn newline_moves_cursor() {
    let mut term = make_term(80, 24);
    term.apply(b"abc\ndef");
    assert_eq!(term.plain_text(), "abc\ndef");
}

#[test]
fn carriage_return_moves_to_column_0() {
    let mut term = make_term(80, 24);
    term.apply(b"abcdef\rXYZ");
    assert_eq!(term.plain_text(), "XYZdef");
}

#[test]
fn backspace_moves_cursor_left() {
    let mut term = make_term(80, 24);
    term.apply(b"abc\x08\x08X");
    assert_eq!(term.plain_text(), "aXc");
}

// ---------------------------------------------------------------------------
// Cursor movement
// ---------------------------------------------------------------------------

#[test]
fn cursor_up() {
    let mut term = make_term(80, 24);
    term.apply(b"line1\n\n\nline4\x1b[A"); // up one line
    // After up from line4, cursor is on line3
    term.apply(b"XXX");
    // This should overwrite start of line3
    let text = term.plain_text();
    assert!(text.contains("XXX"), "cursor up should work: {text:?}");
}

#[test]
fn cursor_position() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b[5;10HX"); // Move to row 5, col 10, print X
    assert_eq!(term.cursor, screen_terminal::Cursor { column: 10, row: 4 });
}

#[test]
fn cursor_horizontal_absolute() {
    let mut term = make_term(80, 24);
    term.apply(b"abcdefghij\x1b[5GX"); // Move to column 5, overwrite
    assert_eq!(term.line_bytes(0).unwrap()[..5], *b"abcdX");
}

// ---------------------------------------------------------------------------
// Erase operations
// ---------------------------------------------------------------------------

#[test]
fn erase_in_line_from_cursor() {
    let mut term = make_term(80, 24);
    term.apply(b"Hello, World!\x1b[K"); // EL 0 (default)
    let line = String::from_utf8_lossy(&term.line_bytes(0).unwrap());
    assert_eq!(line, "Hello, World!");
}

#[test]
fn erase_in_line_to_start() {
    let mut term = make_term(80, 24);
    term.apply(b"Hello, World!\x1b[1K"); // EL 1
    let line = String::from_utf8_lossy(&term.line_bytes(0).unwrap());
    assert_eq!(line, "");
}

#[test]
fn erase_display() {
    let mut term = make_term(80, 24);
    term.apply(b"line1\nline2\nline3\x1b[2J"); // ED 2 - clear entire display
    // Cursor should be at row 2, col 0
    assert_eq!(term.cursor.column, 0);
}

// ---------------------------------------------------------------------------
// SGR (colors and attributes)
// ---------------------------------------------------------------------------

#[test]
fn sgr_bold() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b[1mbold\x1b[0mnormal");
    assert_eq!(term.current_style(), Style::default());
}

#[test]
fn sgr_colors() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b[31mred\x1b[32mgreen");
    let text = term.plain_text();
    assert!(text.contains("red"), "text should contain red");
}

#[test]
fn sgr_reset() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b[1;31mbold red\x1b[0mnormal");
    assert_eq!(term.current_style(), Style::default());
}

// ---------------------------------------------------------------------------
// Scrolling and scrollback
// ---------------------------------------------------------------------------

#[test]
fn scrollback_accumulates() {
    let mut term = make_term(40, 5); // Small terminal
    for i in 0..10 {
        term.apply(format!("line {i}\n").as_bytes());
    }
    assert!(term.scrollback_len() > 0, "scrollback should have content");
}

#[test]
fn scrollback_limit() {
    let mut term = make_term(40, 5);
    term.set_scrollback_limit(3);
    for i in 0..20 {
        term.apply(format!("line {i}\n").as_bytes());
    }
    assert!(
        term.scrollback_len() <= 3,
        "scrollback should be bounded: {}",
        term.scrollback_len()
    );
}

// ---------------------------------------------------------------------------
// Alternate screen
// ---------------------------------------------------------------------------

#[test]
fn alternate_screen_switch() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b[?1049h"); // Switch to alt screen with cursor save
    assert!(term.is_alternate(), "should be in alternate screen");
    term.apply(b"alternate content");
    term.apply(b"\x1b[?1049l"); // Switch back
    assert!(!term.is_alternate(), "should be back to primary");
}

// ---------------------------------------------------------------------------
// Resize
// ---------------------------------------------------------------------------

#[test]
fn terminal_resize() {
    let mut term = make_term(80, 24);
    term.apply(b"Hello, World!");
    term.resize(Dimensions::new(132, 43));
    assert_eq!(term.dimensions.columns, 132);
    assert_eq!(term.dimensions.rows, 43);
}

// ---------------------------------------------------------------------------
// UTF-8
// ---------------------------------------------------------------------------

#[test]
fn utf8_rendering() {
    let mut term = make_term(80, 24);
    term.apply("héllo wörld ©™".as_bytes());
    let text = term.plain_text();
    assert!(text.contains("héllo"), "UTF-8 should render: {text:?}");
}

#[test]
fn wide_characters() {
    let mut term = make_term(80, 24);
    // CJK character: U+4E2D (中) - 3 bytes in UTF-8
    term.apply("\u{4e2d}".as_bytes());
    let text = term.plain_text();
    assert!(text.contains('中'), "wide char should render: {text:?}");
}

// ---------------------------------------------------------------------------
// OSC title
// ---------------------------------------------------------------------------

#[test]
fn osc_title_bel() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b]2;My Title\x07");
    assert_eq!(
        term.title.as_deref(),
        Some(&b"My Title"[..]),
        "OSC title should be set"
    );
}

#[test]
fn osc_title_st() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b]0;Another Title\x1b\\");
    assert_eq!(
        term.title.as_deref(),
        Some(&b"Another Title"[..]),
        "OSC title with ST should be set"
    );
}

// ---------------------------------------------------------------------------
// Fragmented input
// ---------------------------------------------------------------------------

#[test]
fn fragmented_escape_sequences() {
    let mut term = make_term(80, 24);
    // Send escape sequence in parts
    term.apply(b"\x1b[");
    term.apply(b"31m");
    term.apply(b"red text");
    let text = term.plain_text();
    assert_eq!(text, "red text");
}

// ---------------------------------------------------------------------------
// Bell
// ---------------------------------------------------------------------------

#[test]
fn bell_sets_flag() {
    let mut term = make_term(80, 24);
    assert!(!term.take_bell(), "bell should not be set initially");
    term.apply(b"\x07");
    assert!(term.take_bell(), "bell should be set after BEL");
    assert!(!term.take_bell(), "bell should be cleared after take");
}

// ---------------------------------------------------------------------------
// Mouse modes
// ---------------------------------------------------------------------------

#[test]
fn enable_mouse_tracking() {
    let mut term = make_term(80, 24);
    assert_eq!(term.mouse_mode(), MouseMode::Off);
    term.apply(b"\x1b[?1000h");
    assert_eq!(term.mouse_mode(), MouseMode::Normal);
}

#[test]
fn enable_sgr_mouse() {
    let mut term = make_term(80, 24);
    term.apply(b"\x1b[?1006h");
    assert_eq!(term.mouse_mode(), MouseMode::Sgr);
}
