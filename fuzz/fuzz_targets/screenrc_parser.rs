#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz the .screenrc config parser.  Arbitrary (potentially invalid)
    // config text must never panic.
    let _ = screen_config::parse_config(data);
});
