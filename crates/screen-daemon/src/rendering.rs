use crate::{AttachedClient, DaemonError, SessionState, broadcast};
use screen_protocol::{Message, WindowInfoMsg};

pub(crate) fn send_window_list_to_client(
    clients: &mut Vec<AttachedClient>,
    client_id: u64,
    session: &SessionState,
) -> Result<(), DaemonError> {
    // Find the client's selected window number
    let client_selected_num = clients
        .iter()
        .find(|c| c.id == client_id)
        .and_then(|c| session.windows.get(c.selected))
        .map(|w| w.number);

    let list: Vec<WindowInfoMsg> = session
        .windows
        .iter()
        .map(|w| {
            let selected = Some(w.number) == client_selected_num;
            let dead = !w.alive;
            let flags: u8 = if selected && dead {
                3
            } else if selected {
                1
            } else if dead {
                2
            } else {
                0
            };
            WindowInfoMsg {
                number: w.number,
                flags,
                title: w.terminal.title.clone().unwrap_or_default(),
                group: w.group.clone(),
            }
        })
        .collect();

    let mut i = 0;
    while i < clients.len() {
        if clients[i].id == client_id {
            if Message::WindowList(list.clone())
                .write_to(&mut clients[i].stream)
                .is_err()
            {
                clients.remove(i);
            }
            break;
        }
        i += 1;
    }
    Ok(())
}

pub(crate) fn broadcast_region_layout(
    session: &SessionState,
    clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    // When regions are active, render composite view for all clients
    if session.regions.len() <= 1 {
        // Also send region layout metadata for status display
        if !session.regions.is_empty() {
            let layout: Vec<(u32, u16, u16, u16, u16, bool)> = session
                .regions
                .iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    session.windows.get(r.window_idx).map(|w| {
                        (
                            w.number,
                            r.top,
                            r.height,
                            r.left,
                            r.width,
                            i == session.focused_region,
                        )
                    })
                })
                .collect();
            if !layout.is_empty() {
                broadcast(clients, &Message::RegionLayout(layout))?;
            }
        }
        return Ok(());
    }
    // Render composite and send to all clients
    let composite = composite_regions(session);
    broadcast(clients, &Message::PtyOutput(composite))?;
    // Also send region layout metadata
    let layout: Vec<(u32, u16, u16, u16, u16, bool)> = session
        .regions
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            session.windows.get(r.window_idx).map(|w| {
                (
                    w.number,
                    r.top,
                    r.height,
                    r.left,
                    r.width,
                    i == session.focused_region,
                )
            })
        })
        .collect();
    if !layout.is_empty() {
        broadcast(clients, &Message::RegionLayout(layout))?;
    }
    Ok(())
}

/// Render all regions into a composite terminal frame.
fn composite_regions(session: &SessionState) -> Vec<u8> {
    if session.regions.is_empty() {
        return Vec::new();
    }

    let first_window = session.windows.iter().find(|_| true);
    let total_cols = first_window
        .map(|w| w.terminal.dimensions.columns)
        .unwrap_or(80);
    let total_rows = first_window
        .map(|w| w.terminal.dimensions.rows)
        .unwrap_or(24);

    let is_column_split = session.regions[0].width > 0;

    let mut output = Vec::new();
    output.extend_from_slice(b"\x1b[?25l\x1b[H\x1b[J");

    if is_column_split {
        for screen_row in 0..total_rows {
            output.extend_from_slice(b"\x1b[");
            write_usize_buffer(&mut output, screen_row as usize + 1);
            output.extend_from_slice(b";1H");
            for (i, region) in session.regions.iter().enumerate() {
                if let Some(window) = session.windows.get(region.window_idx) {
                    let region_width = region.width.min(total_cols - region.left);
                    if let Some(line) = window.terminal.line_bytes(screen_row) {
                        let row_bytes = line_from_bytes_padded(&line, region_width);
                        output.extend_from_slice(&row_bytes);
                    } else {
                        output.extend(std::iter::repeat_n(b' ', region_width as usize));
                    }
                    if i + 1 < session.regions.len() {
                        output.extend_from_slice(b"\x1b[7m \x1b[0m");
                    }
                }
            }
            output.extend_from_slice(b"\x1b[K");
        }
    } else {
        for (i, region) in session.regions.iter().enumerate() {
            if let Some(window) = session.windows.get(region.window_idx) {
                let rows = window.terminal.dimensions.rows;
                let region_height = region.height.min(rows);
                for row in 0..region_height {
                    let screen_row = region.top + row;
                    output.extend_from_slice(b"\x1b[");
                    write_usize_buffer(&mut output, screen_row as usize + 1);
                    output.extend_from_slice(b";1H");
                    if let Some(line) = window.terminal.line_bytes(row) {
                        output.extend_from_slice(&line);
                    }
                    output.extend_from_slice(b"\x1b[K");
                }
                for row in rows..region.height {
                    let screen_row = region.top + row;
                    output.extend_from_slice(b"\x1b[");
                    write_usize_buffer(&mut output, screen_row as usize + 1);
                    output.extend_from_slice(b";1H\x1b[K");
                }
                if i + 1 < session.regions.len() {
                    let sep_row = region.top + region.height;
                    output.extend_from_slice(b"\x1b[");
                    write_usize_buffer(&mut output, sep_row as usize + 1);
                    output.extend_from_slice(b";1H\x1b[7m");
                    output.extend(std::iter::repeat_n(b'-', total_cols as usize));
                    output.extend_from_slice(b"\x1b[0m");
                }
            }
        }
    }

    if let Some(region) = session.regions.get(session.focused_region)
        && let Some(window) = session.windows.get(region.window_idx)
    {
        let cursor_col = if is_column_split {
            region.left + window.terminal.cursor.column + 1
        } else {
            window.terminal.cursor.column + 1
        };
        let cursor_row = if is_column_split {
            window.terminal.cursor.row + 1
        } else {
            region.top + window.terminal.cursor.row + 1
        };
        output.extend_from_slice(b"\x1b[");
        write_usize_buffer(&mut output, cursor_row as usize);
        output.push(b';');
        write_usize_buffer(&mut output, cursor_col as usize);
        output.push(b'H');
    }

    output.extend_from_slice(b"\x1b[?25h");
    output
}

fn line_from_bytes_padded(line: &[u8], display_width: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(display_width as usize);
    let mut col: u16 = 0;
    let mut i = 0;
    let line_len = line.len();
    while i < line_len && col < display_width {
        if line[i] == 0x1b {
            out.push(0x1b);
            i += 1;
            while i < line_len
                && line[i] != b'm'
                && line[i] != b'H'
                && line[i] != b'J'
                && line[i] != b'K'
                && line[i] != b'A'
                && line[i] != b'B'
                && line[i] != b'C'
                && line[i] != b'D'
                && line[i] != b'h'
                && line[i] != b'l'
            {
                out.push(line[i]);
                i += 1;
            }
            if i < line_len {
                out.push(line[i]);
                i += 1;
            }
        } else {
            out.push(line[i]);
            col += 1;
            i += 1;
        }
    }
    while col < display_width {
        out.push(b' ');
        col += 1;
    }
    out
}

fn write_usize_buffer(output: &mut Vec<u8>, n: usize) {
    if n == 0 {
        output.push(b'0');
        return;
    }
    let mut num = n;
    let mut digits: [u8; 20] = [0; 20];
    let mut pos = 0;
    while num > 0 {
        digits[pos] = (num % 10) as u8 + b'0';
        pos += 1;
        num /= 10;
    }
    for i in (0..pos).rev() {
        output.push(digits[i]);
    }
}

pub(crate) fn send_copy_cursor(
    id: u64,
    session: &SessionState,
    #[allow(clippy::ptr_arg)] clients: &mut Vec<AttachedClient>,
) -> Result<(), DaemonError> {
    if let Some(c) = clients.iter_mut().find(|c| c.id == id)
        && let Some(window) = session.windows.get(c.selected)
    {
        let total = window.scrollback_lines().len() as u32;
        let cursor = session.copy_mode_cursor.min(total.saturating_sub(1));
        let col = (session.copy_mode_column as u16).min(999);
        let _ = Message::CopyModeCursor(cursor, col, total).write_to(&mut c.stream);
    }
    Ok(())
}

#[allow(clippy::ptr_arg)]
pub(crate) fn send_to_client(
    clients: &mut Vec<AttachedClient>,
    client_id: u64,
    message: &Message,
) -> Result<(), DaemonError> {
    let mut i = 0;
    while i < clients.len() {
        if clients[i].id == client_id {
            if message.write_to(&mut clients[i].stream).is_err() {
                clients.remove(i);
            }
            break;
        }
        i += 1;
    }
    Ok(())
}
