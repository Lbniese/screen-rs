use std::time::SystemTime;

/// Expands Screen `%` escape sequences into human-readable text.
///
/// Supported escapes (subset of GNU Screen):
///   %h, %H — hostname
///   %d     — day of month (01–31)
///   %m     — month (01–12)
///   %y     — year (last two digits)
///   %Y     — year (four digits)
///   %c     — current time HH:MM
///   %C     — current time HH:MM:SS
///   %n     — current window number
///   %t     — current window title
///   %w     — window list (numbers, * for current, - for flags)
///   %W     — window list with flags
///   %%     — literal %
///   %=     — right-alignment split (only first = honored)
///
/// Unsupported (passed through unchanged):
///   %? / : — conditionals
///   %{…}   — color/attribute
pub fn expand_hardstatus(
    format: &[u8],
    active_window: u32,
    active_title: &[u8],
    windows: &[WindowInfo],
    now: SystemTime,
    terminal_width: usize,
    backtick_outputs: &std::collections::HashMap<u8, Vec<u8>>,
) -> Vec<u8> {
    let host = hostname();
    let mut out = Vec::with_capacity(format.len() + 32);
    let mut chars = format.iter().copied();
    let mut right_index: Option<usize> = None;

    while let Some(ch) = chars.next() {
        if ch == b'%' {
            let Some(esc) = chars.next() else {
                out.push(b'%');
                break;
            };
            match esc {
                b'h' | b'H' => out.extend_from_slice(host.as_bytes()),
                b'd' => out.extend_from_slice(&fmt_date(now, 3)),
                b'm' => out.extend_from_slice(&fmt_date(now, 4)),
                b'y' => out.extend_from_slice(&fmt_date(now, 5)),
                b'Y' => out.extend_from_slice(&fmt_date(now, 6)),
                b'c' => out.extend_from_slice(&fmt_time(now, false)),
                b'C' => out.extend_from_slice(&fmt_time(now, true)),
                b'w' => out.extend_from_slice(&format_window_list(windows, active_window, false)),
                b'W' => out.extend_from_slice(&format_window_list(windows, active_window, true)),
                b'n' => out.extend_from_slice(format!("{active_window}").as_bytes()),
                b't' => out.extend_from_slice(&escape_title(active_title)),
                b'M' => {
                    // Monitor flag for the active window
                    #[allow(clippy::collapsible_if)]
                    if let Some(w) = windows.iter().find(|w| w.number == active_window) {
                        if w.flags & 4 != 0 {
                            out.extend_from_slice(b"M");
                        }
                    }
                }
                b'=' => {
                    if right_index.is_none() {
                        right_index = Some(out.len());
                    }
                }
                b'%' => out.push(b'%'),
                b'{' => {
                    // skip color/attribute: consume until }
                    for c in chars.by_ref() {
                        if c == b'}' {
                            break;
                        }
                    }
                }
                b'?' => {
                    // Conditional %? … %? — simplified: skip entire construct
                    // Format: %?<cond>%:<true>%? or %?<cond>%:<true>%:<false>%?
                    // We just consume until the matching %? terminator.
                    let mut depth: u32 = 1;
                    while depth > 0 {
                        let Some(c) = chars.next() else {
                            break;
                        };
                        if c == b'%' {
                            let Some(next) = chars.next() else {
                                break;
                            };
                            match next {
                                b'?' => depth += 1,
                                b':' if depth == 1 => {
                                    // Separator: skip the true branch content
                                    // Continue consuming until the closing %?
                                    while depth > 0 {
                                        let Some(cc) = chars.next() else {
                                            break;
                                        };
                                        if cc == b'%' {
                                            let Some(nn) = chars.next() else {
                                                break;
                                            };
                                            match nn {
                                                b'?' => depth -= 1,
                                                b':' if depth == 1 => {
                                                    // false branch found; already
                                                    // consuming, this is just another
                                                    // separator within the skip
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                b'0'..=b'9' => {
                    let idx = esc - b'0';
                    if let Some(output) = backtick_outputs.get(&idx) {
                        out.extend_from_slice(output);
                    }
                }
                _ => {
                    out.push(b'%');
                    out.push(esc);
                }
            }
        } else {
            out.push(ch);
        }
    }

    // Handle right-alignment if %= was used
    if let Some(split) = right_index {
        let left = &out[..split];
        let right = &out[split..];
        let left_len = left.len();
        let right_len = right.len();
        if left_len + right_len < terminal_width {
            let pad = terminal_width.saturating_sub(left_len + right_len);
            let mut aligned: Vec<u8> = Vec::with_capacity(terminal_width + 32);
            aligned.extend_from_slice(left);
            aligned.resize(aligned.len() + pad, b' ');
            aligned.extend_from_slice(right);
            out = aligned;
        }
    }

    out
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub number: u32,
    pub flags: u8,
    pub title: Vec<u8>,
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| String::from("localhost"))
}

fn fmt_date(t: SystemTime, idx: usize) -> Vec<u8> {
    use std::time::UNIX_EPOCH;
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days_since_epoch = secs / 86400;
    let (year, month_idx, day) = civil_from_days(days_since_epoch);
    let year_str = format!("{year}");

    match idx {
        3 => format!("{day:02}").into_bytes(),             // %d
        4 => format!("{:02}", month_idx + 1).into_bytes(), // %m (1-based)
        5 => year_str.as_bytes()[2..].to_vec(),            // %y (last two digits)
        6 => year_str.into_bytes(),                        // %Y
        _ => vec![],
    }
}

fn fmt_time(t: SystemTime, with_seconds: bool) -> Vec<u8> {
    use std::time::UNIX_EPOCH;
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let remaining_secs = secs % 86400;
    let hour = remaining_secs / 3600;
    let min = (remaining_secs % 3600) / 60;
    let sec = remaining_secs % 60;
    if with_seconds {
        format!("{hour:02}:{min:02}:{sec:02}").into_bytes()
    } else {
        format!("{hour:02}:{min:02}").into_bytes()
    }
}

fn format_window_list(windows: &[WindowInfo], active: u32, show_flags: bool) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for (i, w) in windows.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        let marker = if w.number == active {
            '*'
        } else if w.flags & 2 != 0 {
            '@'
        } else {
            '-'
        };
        out.extend_from_slice(format!("{}{}", w.number, marker).as_bytes());
        if show_flags && w.flags != 0 {
            out.extend_from_slice(format!(":f{}", w.flags).as_bytes());
        }
    }
    out
}

fn escape_title(title: &[u8]) -> Vec<u8> {
    // Return title, but truncate at first NUL or newline
    let end = title
        .iter()
        .position(|&b| b == 0 || b == b'\n')
        .unwrap_or(title.len());
    title[..end].to_vec()
}

/// Gregorian calendar from days since Unix epoch (1970-01-01 = day 0).
/// Returns (year, month 0-11, day 1-31).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m - 1, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn test_time() -> SystemTime {
        // 2025-07-14 14:30:42 UTC
        UNIX_EPOCH + Duration::from_secs(1752503442)
    }

    #[test]
    fn hostname_expands() {
        let result = expand_hardstatus(
            b"%H",
            0,
            b"",
            &[],
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        assert!(!result.is_empty());
    }

    #[test]
    fn datetime_expands() {
        let result = expand_hardstatus(
            b"%d/%m/%Y %c",
            0,
            b"",
            &[],
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        let s = String::from_utf8_lossy(&result);
        assert!(s.contains("/2025"), "{s}");
        assert!(s.contains("14:30"), "{s}");
    }

    #[test]
    fn window_list_expands() {
        let wins = [
            WindowInfo {
                number: 0,
                flags: 1,
                title: b"bash".to_vec(),
            },
            WindowInfo {
                number: 1,
                flags: 0,
                title: b"htop".to_vec(),
            },
        ];
        let result = expand_hardstatus(
            b"%w",
            1,
            b"",
            &wins,
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        let s = String::from_utf8_lossy(&result);
        assert_eq!(s, "0- 1*");
    }

    #[test]
    fn literal_percent() {
        let result = expand_hardstatus(
            b"100%%",
            0,
            b"",
            &[],
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        assert_eq!(String::from_utf8_lossy(&result), "100%");
    }

    #[test]
    fn right_align() {
        let result = expand_hardstatus(
            b"left%=right",
            0,
            b"",
            &[],
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        let s = String::from_utf8_lossy(&result);
        assert!(s.starts_with("left"));
        assert!(s.ends_with("right"));
        assert!(s.contains("  "), "expected padding between left and right");
    }

    #[test]
    fn window_number_and_title() {
        let result = expand_hardstatus(
            b"%n %t",
            3,
            b"hello",
            &[],
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        assert_eq!(String::from_utf8_lossy(&result), "3 hello");
    }

    #[test]
    fn skips_color_attributes() {
        let result = expand_hardstatus(
            b"%{=b bc}%n",
            0,
            b"",
            &[],
            test_time(),
            80,
            &std::collections::HashMap::new(),
        );
        assert_eq!(String::from_utf8_lossy(&result), "0");
    }
}
