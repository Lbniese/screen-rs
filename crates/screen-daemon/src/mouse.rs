use crate::{AttachedClient, DaemonError, SessionState, broadcast};
use screen_protocol::Message;

// ---------------------------------------------------------------------------
// Mouse event decoding
// ---------------------------------------------------------------------------

/// Decoded mouse event from a terminal mouse report.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MouseEvent {
    button: u8,    // 0=left, 1=middle, 2=right, 3=release, 4=scroll-up, 5=scroll-down
    column: u16,   // 0-based column
    row: u16,      // 0-based row
    pressed: bool, // true for press/scroll, false for release
}

/// Try to decode a mouse report from the beginning of a byte buffer.
/// Returns (MouseEvent, bytes_consumed) if successful.
pub(crate) fn try_decode_mouse(
    bytes: &[u8],
    mode: screen_terminal::MouseMode,
) -> Option<(MouseEvent, usize)> {
    if bytes.len() < 3 {
        return None;
    }
    match mode {
        screen_terminal::MouseMode::Off => None,
        screen_terminal::MouseMode::Sgr => {
            // SGR: \x1b[<button;col;rowM (press) or \x1b[<button;col;rowm (release)
            if bytes.len() < 6 || &bytes[0..3] != b"\x1b[<" {
                return None;
            }
            let mut params = [0u16; 3];
            let mut param_idx = 0usize;
            let mut pos = 3usize;
            let mut final_byte = 0u8;
            while pos < bytes.len() {
                let b = bytes[pos];
                pos += 1;
                if b == b'M' || b == b'm' {
                    final_byte = b;
                    break;
                } else if b == b';' {
                    if param_idx < 2 {
                        param_idx += 1;
                    }
                } else if b.is_ascii_digit() {
                    let v = params[param_idx] as u32;
                    params[param_idx] = v
                        .saturating_mul(10)
                        .saturating_add((b - b'0') as u32)
                        .min(u16::MAX as u32) as u16;
                } else {
                    return None; // unexpected byte
                }
            }
            if final_byte == 0 {
                return None; // incomplete
            }
            let button = params[0] as u8;
            let (btn, pressed) = decode_sgr_button(button);
            Some((
                MouseEvent {
                    button: btn,
                    column: params[1].saturating_sub(1),
                    row: params[2].saturating_sub(1),
                    pressed,
                },
                pos,
            ))
        }
        _ => {
            // X10 / Normal / ButtonEvent / AnyEvent: \x1b[M <b+32> <c+32> <r+32>
            if bytes.len() < 6 || &bytes[0..3] != b"\x1b[M" {
                return None;
            }
            let button_raw = bytes[3].saturating_sub(0x20);
            let col = bytes[4].saturating_sub(0x20) as u16;
            let row = bytes[5].saturating_sub(0x20) as u16;
            let (btn, pressed) = if button_raw >= 64 {
                // Wheel: buttons 64=up, 65=down
                (button_raw - 60, true) // 4=up, 5=down
            } else if button_raw == 3 {
                // Release in mode 1000+ (button 3 is sentinel for release)
                (0, false)
            } else if button_raw & 32 != 0 {
                // Motion event (mode 1002, 1003)
                (button_raw & 3, true)
            } else {
                (button_raw & 3, true)
            };
            Some((
                MouseEvent {
                    button: btn,
                    column: col,
                    row,
                    pressed,
                },
                6,
            ))
        }
    }
}

/// Decode SGR button encoding.
/// Bits 0-1: button (0=left, 1=middle, 2=right)
/// Bit 6: wheel (add 64)
/// Bit 5: motion (mode 1002/1003)
fn decode_sgr_button(raw: u8) -> (u8, bool) {
    let low = raw & 3;
    if raw >= 64 {
        // Wheel
        (low + 4, true)
    } else if raw & 32 != 0 {
        (low, true) // motion
    } else {
        // Press or release (release has no special marker in SGR; M=press, m=release)
        // But the final byte M/m tells us press/release, handled by caller
        (low, true)
    }
}

/// Handle a decoded mouse event: clicks on hardstatus select windows/regions,
/// other events are forwarded to the active window's pty.
pub(crate) fn handle_mouse_event(
    client_id: u64,
    event: MouseEvent,
    session: &mut SessionState,
    clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    let Some(client) = clients.iter_mut().find(|c| c.id == client_id) else {
        return Ok(());
    };
    let Some(window) = session.windows.get(client.selected) else {
        return Ok(());
    };
    let term_rows = window.terminal.dimensions.rows;
    let term_cols = window.terminal.dimensions.columns;

    // Check if click is on hardstatus line (last row)
    if event.row >= term_rows && session.hardstatus_format.is_some() {
        // Click on hardstatus — interpret as window/region selection
        if event.pressed && event.button == 0 {
            handle_hardstatus_click(client_id, event.column, term_cols, session, clients)?;
        }
        return Ok(());
    }

    // Forward mouse event to pty
    if event.row < term_rows {
        let encoded = encode_mouse_event(&event, window.terminal.mouse_mode());
        if !encoded.is_empty() {
            session.write_to_window(client.selected, &encoded)?;
        }
    }
    Ok(())
}

/// Click on hardstatus: select window by its position in the window list.
fn handle_hardstatus_click(
    _client_id: u64,
    column: u16,
    _term_cols: u16,
    session: &mut SessionState,
    clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    // The hardstatus line format is typically:
    // "left-aligned-content" + padding + "right-aligned-content"
    // Window numbers appear in the left part or as %w / %W list
    // For simplicity, we find the window whose number's position contains the click column

    let status = session.format_hardstatus();
    let status_str = String::from_utf8_lossy(&status);

    // Look for window number patterns in the status: "N*" or "N-"
    let alive: Vec<(u32, usize, usize)> = session
        .windows
        .iter()
        .filter(|w| w.alive)
        .filter_map(|w| {
            let pattern = format!("{}*", w.number);
            status_str
                .find(&pattern)
                .map(|pos| (w.number, pos, pattern.len()))
        })
        .collect();

    // Also check without marker
    let alt: Vec<(u32, usize, usize)> = session
        .windows
        .iter()
        .filter(|w| w.alive)
        .filter_map(|w| {
            let pattern = format!("{}", w.number);
            // Only match if it's a standalone number (preceded by space or at start)
            status_str
                .match_indices(&pattern)
                .find(|(pos, _)| *pos == 0 || status_str.as_bytes().get(pos - 1) == Some(&b' '))
                .map(|(pos, _)| (w.number, pos, pattern.len()))
        })
        .collect();

    // Find the window whose number's column range contains the click
    let all_matches: Vec<_> = alive.iter().chain(alt.iter()).collect();
    for (num, pos, len) in all_matches {
        let start_col = *pos as u16;
        let end_col = start_col + *len as u16;
        if column >= start_col && column < end_col {
            // Select this window
            let new_idx = session.window_index(*num);
            if let Some(idx) = new_idx {
                for client in clients.iter_mut() {
                    client.last_selected = client.selected;
                    client.selected = idx;
                }
                // Redraw and notify
                if let Some(window) = session.windows.get(idx) {
                    let redraw = window.grid_redraw();
                    broadcast(clients, &Message::PtyOutput(redraw))?;
                }
                broadcast(clients, &Message::WindowSelected { number: *num })?;
            }
            break;
        }
    }
    Ok(())
}

/// Encode a mouse event for forwarding to the pty, in the format the pty expects.
fn encode_mouse_event(event: &MouseEvent, mode: screen_terminal::MouseMode) -> Vec<u8> {
    match mode {
        screen_terminal::MouseMode::Sgr => {
            let final_byte = if event.pressed { b'M' } else { b'm' };
            let button = if event.button >= 4 {
                64 + (event.button - 4)
            } else {
                event.button
            };
            format!(
                "\x1b[<{};{};{}{}",
                button,
                event.column + 1,
                event.row + 1,
                final_byte as char
            )
            .into_bytes()
        }
        screen_terminal::MouseMode::Off => Vec::new(),
        _ => {
            // X10/Normal format
            let button_byte = if event.button >= 4 {
                0x20 + 64 + (event.button - 4)
            } else if !event.pressed {
                0x20 + 3 // release sentinel
            } else {
                0x20 + event.button
            };
            vec![
                b'\x1b',
                b'[',
                b'M',
                button_byte,
                event.column.saturating_add(1).min(255) as u8 + 0x20,
                event.row.saturating_add(1).min(255) as u8 + 0x20,
            ]
        }
    }
}
