# Open Compatibility Cases

Generated from the latest Linux/glibc differential run and current local test evidence.

## Active differential mismatches

None currently recorded.

## Platform / lab blockers still observed

| Case ID | Area | Profiles | Note |
|---|---|---|---|
| LAB-001 | GNU Screen attach-or-create create branch | local macOS reference harness only | Linux/glibc GNU Screen 4.9.1 and 5.0.2 now pass this case without PASS/SKIP by using `screenrc` shell configuration plus a file-backed readiness marker when GNU Screen omits PTY-visible shell output. The local macOS reference process can still remain attached after the marker is observed, so the comparison is skipped there as a PTY/reference harness artifact; `screen-rs` completes. |
