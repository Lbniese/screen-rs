# screen-rs

`screen-rs` is an early Rust implementation effort targeting measured,
versioned compatibility with GNU Screen.

This repository is not GNU Screen compatible yet. The current binary is named
`screen-rs` and prints development-only output for unimplemented runtime
operations.

## Build

```sh
cargo build --workspace
```

## Test

```sh
SCREEN_REFERENCE=/usr/bin/screen \
SCREEN_CANDIDATE=target/debug/screen-rs \
cargo test --workspace
```

Some session differential tests create real PTYs and Unix sockets; run them in
an environment that permits those operations for full coverage.

Compatibility claims must be backed by tests and recorded in
`COMPATIBILITY.md`.
