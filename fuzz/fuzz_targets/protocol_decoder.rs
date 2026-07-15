#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz the binary protocol decoder.  Malformed or truncated input must
    // never panic — at worst it should return a ProtocolError.
    let _ = screen_protocol::Message::read_from(&mut std::io::Cursor::new(data));
});
