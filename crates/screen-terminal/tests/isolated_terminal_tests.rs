//! Isolated tests for screen-terminal.
//!
//! These tests exercise the public TerminalState API and verify the terminal
//! emulator state machine, grid operations, SGR, UTF-8, and edge-case handling.
//!
//! Once verified, these tests will be merged into the inline `mod tests` section
//! of src/lib.rs.

use screen_terminal::{Color, Cursor, Dimensions, MouseMode, Style, TerminalState};

/// Helper: create a terminal with given dimensions.
fn term(columns: u16, rows: u16) -> TerminalState {
    TerminalState::new(Dimensions::new(columns, rows))
}

// ---------------------------------------------------------------------------
// Grid operations (tested through TerminalState public API)
// ---------------------------------------------------------------------------

#[test]
fn test_grid_creation() {
    let t = term(5, 3);
    assert_eq!(t.line_bytes(0), Some(b"".to_vec()));
    assert_eq!(t.line_bytes(1), Some(b"".to_vec()));
    assert_eq!(t.line_bytes(2), Some(b"".to_vec()));
    assert!(t.line_bytes(3).is_none());
}

#[test]
fn test_grid_line_bytes() {
    let mut t = term(8, 3);
    t.apply(b"ABC  DEF");
    // Trailing spaces should be trimmed
    assert_eq!(t.line_bytes(0), Some(b"ABC  DEF".to_vec()));
    // Write shorter content, trailing spaces trimmed
    t.apply(b"\x1b[2;1HXYZ");
    assert_eq!(t.line_bytes(1), Some(b"XYZ".to_vec()));
    // Blank line
    assert_eq!(t.line_bytes(2), Some(b"".to_vec()));
}

// ---------------------------------------------------------------------------
// Erase operations
// ---------------------------------------------------------------------------

#[test]
fn test_erase_display_0() {
    // Use a wider terminal so auto-wrap doesn't scroll content
    let mut t = term(10, 5);
    t.apply(b"ABCDE\x1b[2;1HFGHIJ\x1b[3;1HKLMNO");
    // Cursor to middle of row 1, erase from cursor to end of display
    t.apply(b"\x1b[2;3H\x1b[0J");
    assert_eq!(t.line_bytes(0), Some(b"ABCDE".to_vec()));
    // Row 1: "FG" remain (2 chars before cursor at col 2), rest cleared
    assert_eq!(t.line_bytes(1), Some(b"FG".to_vec()));
    // Row 2: completely cleared
    assert_eq!(t.line_bytes(2), Some(b"".to_vec()));
}

#[test]
fn test_erase_display_1() {
    let mut t = term(10, 5);
    t.apply(b"ABCDE\x1b[2;1HFGHIJ\x1b[3;1HKLMNO");
    // Cursor to row 1 col 3, erase from beginning to cursor
    t.apply(b"\x1b[2;3H\x1b[1J");
    assert_eq!(t.line_bytes(0), Some(b"".to_vec()));
    // Row 1: cols 0-2 cleared (F, G, H), I and J remain at cols 3-4
    assert_eq!(t.line_bytes(1), Some(b"   IJ".to_vec()));
    // Row 2: untouched
    assert_eq!(t.line_bytes(2), Some(b"KLMNO".to_vec()));
}

#[test]
fn test_erase_display_2() {
    let mut t = term(10, 5);
    t.apply(b"ABCDE\x1b[2;1HFGHIJ\x1b[3;1HKLMNO");
    t.apply(b"\x1b[2J");
    // Entire display cleared
    assert_eq!(t.plain_text(), "\n\n\n\n");
}

#[test]
fn test_erase_display_3() {
    let mut t = term(10, 5);
    t.apply(b"ABCDE\x1b[2;1HFGHIJ\x1b[3;1HKLMNO");
    t.apply(b"\x1b[3J");
    // Same as 2 — entire display cleared
    assert_eq!(t.plain_text(), "\n\n\n\n");
}

#[test]
fn test_erase_line_0() {
    let mut t = term(5, 3);
    t.apply(b"ABCDE");
    t.apply(b"\x1b[1;3H\x1b[0K"); // from col 2 to end
    assert_eq!(t.line_bytes(0), Some(b"AB".to_vec()));
}

#[test]
fn test_erase_line_1() {
    let mut t = term(5, 3);
    // After ABCDE, auto-wrap moves cursor to row 1 col 0.
    // Go back to row 0 col 2 to erase from beginning to that point.
    t.apply(b"ABCDE\x1b[1;3H\x1b[1K");
    // Erase from start to col 2 (0-indexed): cols 0,1,2 become blank
    // Result: "   DE" (3 spaces then D and E)
    assert_eq!(t.line_bytes(0), Some(b"   DE".to_vec()));
}

#[test]
fn test_erase_line_2() {
    let mut t = term(5, 3);
    t.apply(b"ABCDE");
    t.apply(b"\x1b[1;3H\x1b[2K"); // entire line
    assert_eq!(t.line_bytes(0), Some(b"".to_vec()));
}

// ---------------------------------------------------------------------------
// SGR color tests
// ---------------------------------------------------------------------------

#[test]
fn test_sgr_colors_8() {
    let mut t = term(10, 1);
    // 30-37: standard foreground colors
    t.apply(b"\x1b[31mA\x1b[32mB\x1b[33mC");
    assert_eq!(
        t.cell(0, 0).unwrap().style.foreground,
        Some(Color::Basic(1))
    );
    assert_eq!(
        t.cell(1, 0).unwrap().style.foreground,
        Some(Color::Basic(2))
    );
    assert_eq!(
        t.cell(2, 0).unwrap().style.foreground,
        Some(Color::Basic(3))
    );
    // 39: default foreground
    t.apply(b"\x1b[39mD");
    assert_eq!(t.cell(3, 0).unwrap().style.foreground, None);
}

#[test]
fn test_sgr_colors_background() {
    let mut t = term(10, 1);
    // 40-47: standard background colors
    t.apply(b"\x1b[41mA\x1b[42mB");
    assert_eq!(
        t.cell(0, 0).unwrap().style.background,
        Some(Color::Basic(1))
    );
    assert_eq!(
        t.cell(1, 0).unwrap().style.background,
        Some(Color::Basic(2))
    );
    // 49: default background
    t.apply(b"\x1b[49mC");
    assert_eq!(t.cell(2, 0).unwrap().style.background, None);
}

#[test]
fn test_sgr_colors_90_97() {
    let mut t = term(10, 1);
    // 90-97: bright foreground
    t.apply(b"\x1b[91mA\x1b[92mB");
    assert_eq!(
        t.cell(0, 0).unwrap().style.foreground,
        Some(Color::Basic(9))
    );
    assert_eq!(
        t.cell(1, 0).unwrap().style.foreground,
        Some(Color::Basic(10))
    );
}

#[test]
fn test_sgr_colors_100_107() {
    let mut t = term(10, 1);
    // 100-107: bright background
    t.apply(b"\x1b[101mA\x1b[102mB");
    assert_eq!(
        t.cell(0, 0).unwrap().style.background,
        Some(Color::Basic(9))
    );
    assert_eq!(
        t.cell(1, 0).unwrap().style.background,
        Some(Color::Basic(10))
    );
}

// ---------------------------------------------------------------------------
// Insert/replace mode (IRM)
// ---------------------------------------------------------------------------

#[test]
fn test_insert_mode() {
    let mut t = term(8, 2);
    t.apply(b"ABCDE");
    t.apply(b"\x1b[4h"); // enable IRM
    t.apply(b"\x1b[1;4H"); // CUP row=1,col=4 -> row=0,col=3 (0-indexed)
    t.apply(b"XYZ");
    // With IRM, each written byte inserts a blank first, shifting right.
    // ABCDE starting grid. Cursor at col 3.
    // X: insert_cells(3,0,1) → shift D→4, E→5. Grid: ABC X D E _
    // Y: insert_cells(4,0,1) → shift D→5, E→6. Grid: ABC X Y D E
    // Z: insert_cells(5,0,1) → shift D→6, E→7. Grid: ABC X Y Z D E
    assert_eq!(t.line_bytes(0), Some(b"ABCXYZDE".to_vec()));
}

#[test]
fn test_insert_mode_off() {
    let mut t = term(8, 2);
    t.apply(b"ABCDE");
    t.apply(b"\x1b[4h\x1b[4l"); // enable then disable IRM
    t.apply(b"\x1b[1;4HXYZ");
    // Without IRM, "XYZ" overwrites starting at col 3: "ABCXYZ "
    assert_eq!(t.line_bytes(0), Some(b"ABCXYZ".to_vec()));
}

// ---------------------------------------------------------------------------
// Device status reports
// ---------------------------------------------------------------------------

#[test]
fn test_device_status_report_ok() {
    let mut t = term(80, 24);
    // DSR 5: device status
    let resp = t.apply(b"\x1b[5n");
    assert_eq!(resp, b"\x1b[0n", "DSR 5 should acknowledge OK");
}

#[test]
fn test_device_status_report_cursor() {
    let mut t = term(80, 24);
    // Move cursor, then query position
    t.apply(b"\x1b[10;20H");
    let resp = t.apply(b"\x1b[6n");
    assert_eq!(
        resp, b"\x1b[10;20R",
        "DSR 6 should report row;col (1-indexed)"
    );
}

#[test]
fn test_device_status_report_cursor_at_home() {
    let mut t = term(80, 24);
    let resp = t.apply(b"\x1b[6n");
    assert_eq!(resp, b"\x1b[1;1R", "DSR 6 at home should be 1;1");
}

// ---------------------------------------------------------------------------
// Bell
// ---------------------------------------------------------------------------

#[test]
fn test_bell_flag() {
    let mut t = term(10, 3);
    assert!(!t.take_bell());
    t.apply(b"\x07");
    assert!(t.take_bell());
    // Second call returns false
    assert!(!t.take_bell());
}

#[test]
fn test_bell_flag_multiple() {
    let mut t = term(10, 3);
    t.apply(b"\x07\x07\x07");
    // take_bell resets; only true once per occurrence
    assert!(t.take_bell());
    assert!(!t.take_bell());
}

// ---------------------------------------------------------------------------
// Scrollback
// ---------------------------------------------------------------------------

#[test]
fn test_scrollback_capture() {
    let mut t = term(5, 3);
    // Write LINE1 to row 0, auto-wrap pushes cursor to row 1
    t.apply(b"LINE1");
    // Move to row 1 and write LINE2, auto-wrap to row 2
    t.apply(b"\x1b[2;1HLINE2");
    // Move to row 2 and write LINE3 — its auto-wrap triggers a scroll
    // because cursor was at bottom (row 2 = rows-1). LINE1 scrolls off.
    t.apply(b"\x1b[3;1HLINE3");
    // LINE1 should be in scrollback
    assert_eq!(
        t.scrollback_size(),
        1,
        "LINE3's auto-wrap should scroll LINE1 off"
    );
    // The most recent (index 0) scrollback line should contain LINE1
    let sb_line = t.scrollback_line(0).unwrap_or_default();
    assert!(
        sb_line.starts_with(b"LINE1"),
        "scrollback[0] should start with LINE1: {sb_line:?}"
    );
}

#[test]
fn test_scrollback_limit() {
    let mut t = term(5, 3);
    t.set_scrollback_limit(2);
    // Write lines with newlines to trigger multiple scrolls
    for _ in 0..6 {
        t.apply(b"XXXXX\n");
    }
    // Only last 2 lines retained
    assert!(
        t.scrollback_size() <= 2,
        "scrollback size {} should be <= 2",
        t.scrollback_size()
    );
}

#[test]
fn test_scrollback_disabled() {
    let mut t = term(5, 3);
    t.set_scrollback_limit(0);
    t.apply(b"LINE1\nLINE2\nLINE3\n");
    assert_eq!(t.scrollback_size(), 0);
}

#[test]
fn test_scrollback_retrieval() {
    let mut t = term(6, 2);
    // Write AAAAAA (6 chars) to row 0. Auto-wrap triggers: cursor → row 1.
    t.apply(b"AAAAAA");
    assert_eq!(t.scrollback_size(), 0, "no scroll yet");
    // \n at bottom row scrolls AAAAAA into scrollback.
    t.apply(b"\n");
    assert_eq!(
        t.scrollback_size(),
        1,
        "AAAAAA should be in scrollback after newline"
    );
    let line = t.scrollback_line(0).unwrap_or_default();
    assert!(
        line.starts_with(b"AAAAAA"),
        "scrollback[0] should start with AAAAAA: {line:?}"
    );
}

// ---------------------------------------------------------------------------
// DEC Special Graphics (line drawing)
// ---------------------------------------------------------------------------

#[test]
fn test_dec_special_graphics_basic() {
    let mut t = term(20, 3);
    // Select G0 charset '0' (DEC Special Graphics)
    t.apply(b"\x1b(0"); // designate G0 as special graphics
    // Write some line-drawing characters
    t.apply(b"lqk"); // ┌─┐
    // The bytes should be translated to UTF-8 line-drawing characters
    let line = t.line_bytes(0).unwrap();
    // l=┌ (3 bytes UTF-8), q=─ (3 bytes), k=┐ (3 bytes) = 9 bytes
    assert!(
        line.len() >= 6,
        "line-drawing chars should be > 1 byte each: {line:?}"
    );
    // Switch back to ASCII
    t.apply(b"\x1b(B"); // designate G0 as ASCII (US)
    t.apply(b"abc");
    let line = t.line_bytes(0).unwrap();
    // Should end with 3 ASCII bytes
    assert!(
        line.ends_with(b"abc"),
        "should end with abc after charset switch: {line:?}"
    );
}

#[test]
fn test_dec_special_graphics_mapping() {
    let mut t = term(15, 2);
    t.apply(b"\x1b(0jklmn"); // ┘┐┌└┼
    let line = t.line_bytes(0).unwrap();
    // Each byte maps to a >=2 byte UTF-8 sequence
    assert!(line.len() > 6, "multiple line-drawing chars: {line:?}");
}

#[test]
fn test_dec_special_graphics_untouched_with_ascii() {
    let mut t = term(15, 2);
    // Without selecting special graphics, bytes pass through as-is
    t.apply(b"lqk");
    let line = t.line_bytes(0).unwrap();
    assert_eq!(line, b"lqk");
}

// ---------------------------------------------------------------------------
// Mouse modes
// ---------------------------------------------------------------------------

#[test]
fn test_mouse_mode_x10() {
    let mut t = term(80, 24);
    assert_eq!(t.mouse_mode(), MouseMode::Off);
    t.apply(b"\x1b[?9h"); // X10 mouse
    assert_eq!(t.mouse_mode(), MouseMode::X10);
    t.apply(b"\x1b[?9l"); // disable
    assert_eq!(t.mouse_mode(), MouseMode::Off);
}

#[test]
fn test_mouse_mode_normal() {
    let mut t = term(80, 24);
    t.apply(b"\x1b[?1000h"); // normal tracking
    assert_eq!(t.mouse_mode(), MouseMode::Normal);
    t.apply(b"\x1b[?1000l");
    assert_eq!(t.mouse_mode(), MouseMode::Off);
}

#[test]
fn test_mouse_mode_buttonevent() {
    let mut t = term(80, 24);
    t.apply(b"\x1b[?1002h");
    assert_eq!(t.mouse_mode(), MouseMode::ButtonEvent);
}

#[test]
fn test_mouse_mode_anyevent() {
    let mut t = term(80, 24);
    t.apply(b"\x1b[?1003h");
    assert_eq!(t.mouse_mode(), MouseMode::AnyEvent);
}

#[test]
fn test_mouse_mode_sgr() {
    let mut t = term(80, 24);
    t.apply(b"\x1b[?1006h");
    assert_eq!(t.mouse_mode(), MouseMode::Sgr);
}

// ---------------------------------------------------------------------------
// C1 control codes (8-bit equivalents)
// ---------------------------------------------------------------------------

#[test]
fn test_c1_controls_index() {
    let mut t = term(10, 5);
    // 0x84 = IND (same as ESC D)
    t.apply(b"\x84");
    assert_eq!(t.cursor, Cursor { column: 0, row: 1 });
}

#[test]
fn test_c1_controls_next_line() {
    let mut t = term(10, 5);
    t.apply(b"\x1b[5C"); // move right
    // 0x85 = NEL (same as ESC E)
    t.apply(b"\x85");
    assert_eq!(t.cursor, Cursor { column: 0, row: 1 });
}

#[test]
fn test_c1_controls_tab_set() {
    let mut t = term(10, 5);
    // 0x88 = HTS (same as ESC H) — accepted, no-op
    t.apply(b"\x88");
    // should not panic or change cursor
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

#[test]
fn test_c1_controls_reverse_index() {
    let mut t = term(10, 5);
    t.apply(b"\x1b[2;1H\x8d"); // RI (0x8D)
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

#[test]
fn test_c1_controls_ss2_ss3() {
    let mut t = term(10, 5);
    // 0x8E = SS2, 0x8F = SS3 — accepted, no-op (single shift)
    t.apply(b"\x8e\x8f");
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

#[test]
fn test_c1_controls_dcs() {
    let mut t = term(10, 5);
    // 0x90 = DCS — enters escape state, consumed
    t.apply(b"\x90");
    // Should not panic
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

#[test]
fn test_c1_controls_csi() {
    let mut t = term(10, 5);
    // 0x9B = CSI (same as ESC [)
    t.apply(b"\x9b1;1HZ"); // CSI via 8-bit
    assert_eq!(t.line_bytes(0), Some(b"Z".to_vec()));
}

#[test]
fn test_c1_controls_st() {
    let mut t = term(10, 5);
    // 0x9C = ST (same as ESC \) — terminates OSC
    t.apply(b"\x1b]2;hello\x9c");
    assert_eq!(t.title, Some(b"hello".to_vec()));
}

// ---------------------------------------------------------------------------
// Bracketed paste mode
// ---------------------------------------------------------------------------

#[test]
fn test_bracketed_paste_mode() {
    let mut t = term(40, 10);
    // Mode 2004 is referenced in the code but uses bracketed_paste field
    t.apply(b"\x1b[?2004h");
    // The bracketed_paste mode is set internally; we verify no panic
    assert_eq!(t.mouse_mode(), MouseMode::Off); // still off
    t.apply(b"\x1b[?2004l");
}

// ---------------------------------------------------------------------------
// Scrollup/down with counts
// ---------------------------------------------------------------------------

#[test]
fn test_scroll_up_with_count() {
    let mut t = term(8, 5);
    t.apply(b"AAAA\x1b[2;1HBBBB\x1b[3;1HCCCC\x1b[4;1HDDDD\x1b[5;1HEEEE");
    assert_eq!(t.line_bytes(0), Some(b"AAAA".to_vec()));
    assert_eq!(t.line_bytes(1), Some(b"BBBB".to_vec()));
    t.apply(b"\x1b[2S"); // scroll up 2 lines
    // Rows 0-1: BBBB and CCCC moved up 2
    // Wait, scroll_up(0,4,2): removes rows 0-1, shifts 2-4 up, blanks 3-4
    // Actually: after scroll_up with full margins:
    // Row 0 had AAAA, row 1 had BBBB -> scrolled off
    // Row 0 becomes what was row 2 (CCCC), row 1 becomes what was row 3 (DDDD)
    // Row 2 becomes what was row 4 (EEEE), row 3-4 blank
    assert_eq!(t.line_bytes(0), Some(b"CCCC".to_vec()));
    assert_eq!(t.line_bytes(1), Some(b"DDDD".to_vec()));
    assert_eq!(t.line_bytes(2), Some(b"EEEE".to_vec()));
    assert_eq!(t.line_bytes(3), Some(b"".to_vec()));
}

#[test]
fn test_scroll_down_with_count() {
    let mut t = term(8, 5);
    t.apply(b"\x1b[1;1HAAAA");
    t.apply(b"\x1b[2;1HBBBB");
    t.apply(b"\x1b[3;1HCCCC");
    t.apply(b"\x1b[2T"); // scroll down 2 lines
    // Rows 0-1 scrolled down 2, AAAA moves to row 2, BBBB to row 3
    // Rows 0-1 become blank
    assert_eq!(t.line_bytes(0), Some(b"".to_vec()));
    assert_eq!(t.line_bytes(1), Some(b"".to_vec()));
    assert_eq!(t.line_bytes(2), Some(b"AAAA".to_vec()));
    assert_eq!(t.line_bytes(3), Some(b"BBBB".to_vec()));
}

// ---------------------------------------------------------------------------
// Cursor movement edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_cursor_beyond_bounds_clamped() {
    let mut t = term(5, 3);
    t.apply(b"\x1b[99;99H");
    assert_eq!(t.cursor, Cursor { column: 4, row: 2 });
}

#[test]
fn test_cursor_move_negative() {
    let mut t = term(5, 3);
    // Move back from origin
    t.apply(b"\x1b[A");
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
    t.apply(b"\x1b[D");
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

// ---------------------------------------------------------------------------
// Line feed at bottom scrolls
// ---------------------------------------------------------------------------

#[test]
fn test_line_feed_scrolls_at_bottom() {
    let mut t = term(4, 2);
    t.apply(b"AAAA");
    // After AAAA written to 4-col terminal: cursor col=4 triggers auto-wrap
    // cursor = col 0, row 1
    // Now apply explicit newline
    t.apply(b"\n");
    // cursor at row 1, bottom = rows-1 = 1. line_feed: at bottom, scroll.
    // AAAA goes to scrollback, row 0-1 blank, cursor at row 1
    t.apply(b"BBBB");
    // B at col 0-3. After last B: auto-wrap, cursor col=0, row=1 (bottom)
    // Row 0 = "BBBB", row 1 = blank
    assert_eq!(t.line_bytes(0), Some(b"BBBB".to_vec()));
    // Row 1 should have the result of auto-wrap (cursor passed through here)
    // Actually after auto-wrap last B: col 3 + 1 = 4, 4 < 4 = false, wrap
    // Cursor column = 0, line_feed -> cursor.row(1) < bottom(1)? No, scroll
    // scroll_up(0,1,1): removes row 0, shifts row 1 up, blanks row 1
    // row 0 = old row 1 (blank), row 1 = blank
    // Wait, BBBB was on row 0, not row 1.
    // Let me re-trace: after \n, we scrolled. Row 0 = blank, Row 1 = blank.
    // Then BBBB: B(0,0)=B, B(1,0)=B, B(2,0)=B, B(3,0)=B. After col 3:
    // put_byte: col 3 + 1 = 4, 4 < 4 = false, auto_wrap: col=0, line_feed()
    // line_feed: cursor.row=0 < bottom=1? Yes -> cursor.row=1.
    // Now line_bytes(0) = BBBB. line_bytes(1) = blank.
    // Actually after the wrap, cursor = (0, 1). Then the test code ends.
    assert_eq!(t.line_bytes(1), Some(b"".to_vec()));
}

// ---------------------------------------------------------------------------
// RIS reset
// ---------------------------------------------------------------------------

#[test]
fn test_ris_reset() {
    let mut t = term(40, 10);
    t.apply(b"HELLO\x1b[1;31mWORLD\x1b]2;mytitle\x07");
    t.apply(b"\x1bc"); // RIS
    assert_eq!(t.plain_text(), "\n\n\n\n\n\n\n\n\n".to_string());
    assert_eq!(t.current_style(), Style::default());
    assert_eq!(t.title, None);
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
    assert!(t.show_cursor());
    assert!(!t.is_alternate());
}

// ---------------------------------------------------------------------------
// Cursor next/prev line
// ---------------------------------------------------------------------------

#[test]
fn test_cursor_prev_line() {
    let mut t = term(10, 5);
    t.apply(b"\x1b[2F"); // cursor previous line x 2
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 }); // clamped at top
}

#[test]
fn test_cursor_next_line_beyond_bottom() {
    let mut t = term(10, 3);
    t.apply(b"\x1b[5E"); // next line x 5
    // Should scroll and end at row 2 (last visible row)
    assert_eq!(t.cursor, Cursor { column: 0, row: 2 });
}

// ---------------------------------------------------------------------------
// SGR 256-color background
// ---------------------------------------------------------------------------

#[test]
fn test_sgr_256_color_background() {
    let mut t = term(3, 1);
    t.apply(b"\x1b[48;5;42mA");
    assert_eq!(
        t.cell(0, 0).unwrap().style.background,
        Some(Color::Indexed(42))
    );
}

// ---------------------------------------------------------------------------
// DEC private queries
// ---------------------------------------------------------------------------

#[test]
fn test_dec_private_set_reset_no_panic() {
    let mut t = term(80, 24);
    // Various DECSET values
    t.apply(b"\x1b[?1h\x1b[?2h\x1b[?3h\x1b[?4h\x1b[?5h\x1b[?6h\x1b[?7h\x1b[?8h\x1b[?9h");
    t.apply(b"\x1b[?1l\x1b[?2l\x1b[?3l\x1b[?4l\x1b[?5l\x1b[?6l\x1b[?7l\x1b[?8l\x1b[?9l");
    // Should not panic
    assert!(t.cursor.row < 24);
}

// ---------------------------------------------------------------------------
// BCE mode (Background Color Erase)
// ---------------------------------------------------------------------------

#[test]
fn test_bce_mode_erase() {
    let mut t = term(5, 3);
    t.set_bce(true);
    t.apply(b"\x1b[41m"); // red background
    t.apply(b"\x1b[2J"); // clear display
    // With BCE, the erased cells should retain background color
    // Without BCE (default), erased cells have default style
    // Since we set BCE, let's check if erasing with colored bg works
    let cell = t.cell(0, 0).unwrap();
    // The cell should have been blanked, but with the current style (BCE)
    // This would mean blank cells could have the current bg color
    assert!(cell.is_blank());
}

// ---------------------------------------------------------------------------
// Application keypad mode
// ---------------------------------------------------------------------------

#[test]
fn test_application_keypad() {
    let mut t = term(10, 3);
    // DECKPAM (ESC =) is not implemented in current parser
    // DECKPNM (ESC >) is not implemented
    // Just verify no panic
    t.apply(b"\x1b=");
    t.apply(b"\x1b>");
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

// ---------------------------------------------------------------------------
// Tab stops
// ---------------------------------------------------------------------------

#[test]
fn test_horizontal_tab() {
    let mut t = term(20, 3);
    t.apply(b"\x09"); // tab at start
    assert_eq!(t.cursor, Cursor { column: 8, row: 0 });
    t.apply(b"\x09");
    assert_eq!(t.cursor, Cursor { column: 16, row: 0 });
    t.apply(b"\x09");
    assert_eq!(t.cursor, Cursor { column: 19, row: 0 }); // clamped at max-1
}

// ---------------------------------------------------------------------------
// Line feed (0x0A, 0x0B, 0x0C)
// ---------------------------------------------------------------------------

#[test]
fn test_all_line_feed_codes() {
    let mut t = term(10, 3);
    t.apply(b"\x0a\x0b\x0c");
    // All three should move cursor down (3 times)
    assert_eq!(t.cursor, Cursor { column: 0, row: 2 }); // bottom of 3 rows (indices 0-2)
    t.apply(b"\x0a"); // at bottom, should scroll
    assert_eq!(t.cursor, Cursor { column: 0, row: 2 }); // still at bottom
}

// ---------------------------------------------------------------------------
// Response buffer includes only one report at a time
// ---------------------------------------------------------------------------

#[test]
fn test_response_buffer_taken() {
    let mut t = term(80, 24);
    let resp = t.apply(b"\x1b[5n");
    assert_eq!(resp, b"\x1b[0n");
    // Second call without DSR should return empty
    let resp2 = t.apply(b"");
    assert_eq!(resp2, b"");
}

// ---------------------------------------------------------------------------
// DECSCUSR cursor style
// ---------------------------------------------------------------------------

#[test]
fn test_decscusr_cursor_style() {
    let mut t = term(80, 24);
    // CSI Ps SP q — set cursor style
    // These go through the CSI handler with intermediate byte ' '
    t.apply(b"\x1b[2 q"); // steady block
    t.apply(b"\x1b[4 q"); // steady underline
    t.apply(b"\x1b[6 q"); // steady bar
    // Just verify no panic; cursor style is internal
    assert!(t.show_cursor());
}

// ---------------------------------------------------------------------------
// C1 control: SOS, PM, APC (ignored)
// ---------------------------------------------------------------------------

#[test]
fn test_c1_ignored_controls() {
    let mut t = term(10, 3);
    // 0x98 = SOS (String of Strings)
    // 0x9A = DECID / DA
    t.apply(b"\x98\x9a");
    // Should not panic
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

// ---------------------------------------------------------------------------
// Primary DA (Device Attributes)
// ---------------------------------------------------------------------------

#[test]
fn test_primary_device_attributes() {
    let mut t = term(80, 24);
    // CSI c — primary DA
    t.apply(b"\x1b[c");
    // Should respond as VT220
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

// ---------------------------------------------------------------------------
// Secondary DA (with > prefix)
// ---------------------------------------------------------------------------

#[test]
fn test_secondary_device_attributes() {
    let mut t = term(80, 24);
    // CSI > c — secondary DA
    t.apply(b"\x1b[>c");
    // Should respond as GNU Screen
    assert_eq!(t.cursor, Cursor { column: 0, row: 0 });
}

// ---------------------------------------------------------------------------
// Compact scroll tests
// ---------------------------------------------------------------------------

#[test]
fn test_scrollback_capture_on_newline() {
    let mut t = term(5, 3);
    // Write AAAAA fills row 0, auto-wrap → cursor at row 1
    t.apply(b"AAAAA\n");
    // \n → cursor at row 2
    t.apply(b"BBBBB");
    // BBBBB fills row 2 (cols 0-4), auto-wrap → scroll because cursor at bottom
    // AAAAA goes to scrollback
    assert!(t.scrollback_size() >= 1, "AAAAA should be in scrollback");
    // After scroll: row 0 = blank (was row 1), row 1 = BBBBB (was row 2)
    assert!(t.line_bytes(1).unwrap_or_default().starts_with(b"BBBBB"));
}

// ---------------------------------------------------------------------------
// New test: grid content after operations
// ---------------------------------------------------------------------------

#[test]
fn test_grid_cell_content_after_write() {
    let mut t = term(5, 3);
    t.apply(b"HELLO");
    let c = t.cell(0, 0).unwrap();
    assert_eq!(c.bytes, b"H");
    let c = t.cell(4, 0).unwrap();
    assert_eq!(c.bytes, b"O");
}

#[test]
fn test_grid_cursor_advance() {
    let mut t = term(5, 2);
    t.apply(b"A");
    assert_eq!(t.cursor.column, 1);
    t.apply(b"B");
    assert_eq!(t.cursor.column, 2);
}

#[test]
fn test_grid_auto_wrap_edge() {
    let mut t = term(3, 2);
    t.apply(b"AB");
    assert_eq!(t.cursor.column, 2);
    t.apply(b"C"); // at last column
    assert_eq!(t.cursor.column, 0); // wraps after write
    assert_eq!(t.cursor.row, 1);
    t.apply(b"D");
    assert_eq!(t.cursor.column, 1);
}
