# Changelog

## Unreleased

- Initialized the Rust workspace and compatibility test foundation.
- Added PTY test support and a minimal versioned daemon socket skeleton.
- Added a development-only one-window detached PTY session path with listing,
  snapshot attach, and candidate `-X quit`.
- Added candidate resize protocol plumbing and a PTY `SIGWINCH` regression test.
- Added candidate `-X detach`, `-p 0 -X stuff`, Screen-style child
  `STY`/`WINDOW`/`TERM` environment setup, `-T term`, and `-s shell`
  handling.
- Switched candidate session sockets to GNU-style `pid.session` names and
  aligned single-session `-ls`/`-wipe` output and status with observed GNU
  Screen behavior.
- Added the first byte-oriented terminal-state parser with tests for control
  bytes, cursor movement, erase sequences, OSC title parsing, SGR attributes,
  fragmented escape sequences, and arbitrary byte safety.
- Added GNU Screen differential tests for detached lifecycle, child
  environment, terminal override, child-exit cleanup, session listing/wipe, and
  remote `stuff`.
- Added GNU Screen differential coverage for `-s shell` detached startup.
- Added a byte-oriented startup config parser and verified `-c` support for the
  `shell` and `term` commands against GNU Screen.
- Added verified one-window `-L` logging to the default `screenlog.0` path.
- Added startup config support for `chdir`, bounded `source` includes, `log`,
  `deflog`, and `logfile`, plus loading from `SCREENRC` and an existing
  `$HOME/.screenrc`.
- Added parser support for common compact detached-create spellings including
  `-dm`, `-dmS name`, `-dmSname`, and `-DmSname`.
- Added development-only attached create support for one-window PTY sessions
  using the existing daemon and attach path.
- Added development-only `-R`/`-RR` attach-or-create runtime support for the
  no-match create path and exact single-match attach path.
- Added optional session-name filters for `-ls [match]` and `-wipe [match]`.
- Added GNU Screen differential cases for compact detached create, config
  `source`, config `chdir`, `SCREENRC`-driven logging, default
  `$HOME/.screenrc` startup settings, attached create, and attach-or-create
  create behavior, and filtered session listing.
