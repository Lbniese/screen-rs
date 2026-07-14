/// Character encoding conversion for PTY I/O.
///
/// Supports UTF-8 (passthrough), Latin-1 (ISO-8859-1), and ASCII.
/// Other encodings are passed through as raw bytes with a warning.
/// Convert bytes from the PTY encoding to UTF-8 for terminal output.
pub fn pty_to_utf8(data: &[u8], encoding: Option<&[u8]>) -> Vec<u8> {
    let enc = encoding.and_then(|e| std::str::from_utf8(e).ok());
    match enc {
        Some("UTF-8") | Some("utf-8") | Some("utf8") | None => data.to_vec(),
        Some("ISO-8859-1") | Some("iso-8859-1") | Some("latin1") | Some("latin-1") => {
            // Latin-1: each byte 0x80-0xFF maps to Unicode U+0080-U+00FF
            let mut out = Vec::with_capacity(data.len() * 2);
            for &b in data {
                if b < 0x80 {
                    out.push(b);
                } else {
                    // Encode as UTF-8 2-byte sequence for U+0080-U+00FF
                    out.push(0xC2 | (b >> 6));
                    out.push(0x80 | (b & 0x3F));
                }
            }
            out
        }
        Some("ASCII") | Some("ascii") | Some("us-ascii") => {
            // Strip high bit
            data.iter().map(|&b| b & 0x7F).collect()
        }
        _ => {
            // Unknown encoding: pass through
            data.to_vec()
        }
    }
}

/// Convert UTF-8 input from terminal to the PTY encoding.
pub fn utf8_to_pty(data: &[u8], encoding: Option<&[u8]>) -> Vec<u8> {
    let enc = encoding.and_then(|e| std::str::from_utf8(e).ok());
    match enc {
        Some("UTF-8") | Some("utf-8") | Some("utf8") | None => data.to_vec(),
        Some("ISO-8859-1") | Some("iso-8859-1") | Some("latin1") | Some("latin-1") => {
            // UTF-8 to Latin-1: multi-byte sequences -> single byte
            let s = String::from_utf8_lossy(data);
            s.chars()
                .map(|ch| {
                    let cp = ch as u32;
                    if cp <= 0xFF { cp as u8 } else { b'?' }
                })
                .collect()
        }
        Some("ASCII") | Some("ascii") | Some("us-ascii") => {
            data.iter().map(|&b| b & 0x7F).collect()
        }
        _ => data.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_passthrough() {
        let input = b"Hello, \xe2\x82\xac!"; // "Hello, €!"
        assert_eq!(pty_to_utf8(input, Some(b"UTF-8")), input);
    }

    #[test]
    fn latin1_to_utf8() {
        // 0xC0 = À in Latin-1
        assert_eq!(
            pty_to_utf8(b"\xC0 la mode", Some(b"ISO-8859-1")),
            "\u{00C0} la mode".as_bytes()
        );
    }

    #[test]
    fn utf8_to_latin1() {
        assert_eq!(
            utf8_to_pty("café".as_bytes(), Some(b"ISO-8859-1")),
            b"caf\xE9"
        );
    }

    #[test]
    fn ascii_strips_high_bit() {
        assert_eq!(pty_to_utf8(b"\x80\xFF", Some(b"ASCII")), b"\x00\x7F");
    }
}
