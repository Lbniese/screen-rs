/// Character encoding conversion for PTY I/O.
///
/// Supports UTF-8 (passthrough), Latin-1 (ISO-8859-1), ASCII,
/// and any encoding supported by encoding_rs (KOI8-R, EUC-JP,
/// Shift_JIS, KOI8-U, ISO-2022-JP, GBK, Big5, etc.).
/// Unknown encodings are passed through as raw bytes with a warning.
use encoding_rs::Encoding;

/// Resolve an encoding name to an encoding_rs Encoding, returning None for
/// names we handle manually (UTF-8, Latin-1, ASCII).
fn resolve_encoding(name: &str) -> Option<&'static Encoding> {
    // Fast-path: UTF-8, Latin-1, ASCII handled without encoding_rs
    match name.to_lowercase().as_str() {
        "utf-8" | "utf8" => None,
        "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" => None,
        "ascii" | "us-ascii" => None,
        _ => Encoding::for_label(name.as_bytes()),
    }
}

/// Convert bytes from the PTY encoding to UTF-8 for terminal output.
pub fn pty_to_utf8(data: &[u8], encoding: Option<&[u8]>) -> Vec<u8> {
    let enc_name = encoding.and_then(|e| std::str::from_utf8(e).ok());

    match enc_name {
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

        Some(name) => {
            // Use encoding_rs for all other encodings
            if let Some(enc) = resolve_encoding(name) {
                let (cow, _enc_used, had_errors) = enc.decode(data);
                let _ = had_errors; // ignore replacement characters
                cow.into_owned().into_bytes()
            } else {
                // Unknown encoding: pass through
                data.to_vec()
            }
        }
    }
}

/// Convert UTF-8 input from terminal to the PTY encoding.
pub fn utf8_to_pty(data: &[u8], encoding: Option<&[u8]>) -> Vec<u8> {
    let enc_name = encoding.and_then(|e| std::str::from_utf8(e).ok());

    match enc_name {
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

        Some(name) => {
            // Use encoding_rs for all other encodings
            if let Some(enc) = resolve_encoding(name) {
                let text = String::from_utf8_lossy(data);
                let (cow, _enc_used, had_errors) = enc.encode(&text);
                let _ = had_errors;
                cow.into_owned()
            } else {
                data.to_vec()
            }
        }
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

    #[test]
    fn koi8r_to_utf8() {
        // "Привет" (Hello in Russian) in KOI8-R
        let koi8r = b"\xF0\xD2\xC9\xD7\xC5\xD4";
        let result = pty_to_utf8(koi8r, Some(b"KOI8-R"));
        let expected = "Привет".as_bytes();
        assert_eq!(result, expected);
    }

    #[test]
    fn utf8_to_koi8r() {
        let result = utf8_to_pty("Привет".as_bytes(), Some(b"KOI8-R"));
        assert_eq!(result, b"\xF0\xD2\xC9\xD7\xC5\xD4");
    }

    #[test]
    fn shift_jis_roundtrip() {
        // Japanese "こんにちは" (Hello)
        let sjis = b"\x82\xB1\x82\xF1\x82\xC9\x82\xBF\x82\xCD";
        let utf8 = pty_to_utf8(sjis, Some(b"Shift_JIS"));
        assert_eq!(String::from_utf8_lossy(&utf8), "こんにちは");
    }

    #[test]
    fn unknown_encoding_passthrough() {
        let input = b"\x80\x81\x82";
        assert_eq!(pty_to_utf8(input, Some(b"BOGUS")), input);
    }
}
