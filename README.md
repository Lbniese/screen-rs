# screen-rs

`screen-rs` is an early Rust implementation effort targeting measured,
versioned compatibility with GNU Screen.

This repository does not claim full GNU Screen compatibility yet. The current
binary is named `screen-rs`; compatibility is developed and verified
incrementally through versioned feature manifests and differential tests.

## Build

```sh
cargo build --workspace
```

## Install GNU Screen reference builds

Build side-by-side reference binaries locally:

```sh
./scripts/install-screen-reference.sh 4.9.1
./scripts/install-screen-reference.sh 5.0.2
```

This installs to:

- `.local/screen-4.9.1/bin/screen`
- `.local/screen-5.0.2/bin/screen`

## Test

Run the full workspace against one reference:

```sh
SCREEN_REFERENCE=.local/screen-4.9.1/bin/screen \
SCREEN_CANDIDATE=target/debug/screen-rs \
cargo test --workspace
```

Run the differential matrix and write reports under `compatibility/reports/`:

```sh
./scripts/run-differential-matrix.sh 4.9.1 5.0.2
```

Run the Linux/glibc containerized matrix:

```sh
./docker/linux-glibc/run-matrix.sh
```

Some session differential tests create real PTYs and Unix sockets; run them in
an environment that permits those operations for full coverage.

Compatibility claims must be backed by tests and recorded in
`COMPATIBILITY.md`.
