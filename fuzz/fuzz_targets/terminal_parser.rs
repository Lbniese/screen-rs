#![no_main]

use libfuzzer_sys::fuzz_target;
use screen_terminal::{Dimensions, TerminalState};

fuzz_target!(|data: &[u8]| {
    // Fuzz the terminal parser with arbitrary byte sequences.
    // The parser must never panic, OOM, or exhibit UB regardless of input.
    let mut term = TerminalState::new(Dimensions {
        columns: 80,
        rows: 24,
    });
    let _ = term.apply(data);
});
