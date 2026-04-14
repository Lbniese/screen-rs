# Contributing

Keep changes small and testable. GNU Screen is the behavioral reference, and no
compatibility claim should be made without an automated comparison.

Before submitting a change, run:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```
