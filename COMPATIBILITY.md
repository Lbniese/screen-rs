# Compatibility

No GNU Screen compatibility profile is claimed yet.

## Current Development Status

- CLI parsing exists for an initial option subset, including common detached
  create spellings such as `-dm`, `-dmS name`, and `-dmSname`.
- PTY-backed child process tests exist for the test harness.
- A candidate-only daemon socket skeleton exists for hello/shutdown protocol
  experiments.
- Development-only detached PTY sessions can be started with
  `screen-rs -S name -d -m command`.
- Development-only attached one-window PTY sessions can be started with
  `screen-rs [-S name] [command]`; the attached client uses the existing
  daemon path so default `C-a d` detach can leave the session alive.
- Development-only `-R`/`-RR` attach-or-create support exists for the
  one-window daemon: it attaches when exactly one matching active session is
  present and creates a new attached session when no match exists. Aggressive
  multi-session selection semantics are not yet implemented.
- Development-only session discovery uses GNU-style `pid.session` socket names
  for the single-window candidate daemon. `-ls` and `-wipe` now match the
  locally observed GNU Screen output shape and exit status for empty and active
  single-session cases, with optional session-name filtering accepted for
  `-ls [match]` and `-wipe [match]`.
- Development-only snapshot attach and `-X quit` exist for candidate daemons.
- Development-only `-X detach` and `-p 0 -X stuff` exist for the single-window
  candidate daemon.
- Detached child environment now matches tested GNU Screen behavior for
  `STY=<pid>.<session>`, `WINDOW=0`, default `TERM=screen`, and `-T term`.
- Detached startup honors tested GNU Screen `-s shell` behavior when no explicit
  command is supplied.
- Startup config loading supports explicit `-c file`, `SCREENRC`, and an
  existing `$HOME/.screenrc`.
- `-c file`/startup config parsing is implemented for a minimal tested startup
  subset: `shell <path>`, `term <name>`, `chdir <path>`, absolute or
  config-relative `source <file>`, `log on/off`, `deflog on/off`, and
  `logfile <path>`. Full `.screenrc` compatibility is not claimed.
- `-L` is implemented for the tested one-window detached case, writing raw PTY
  output to the default `screenlog.0` file in the launcher working directory.
  Startup config logging can also enable one-window detached logging with
  `deflog on`/`log on` and `logfile <path>`.
- PTY resize is implemented in the PTY layer and candidate protocol; child
  `SIGWINCH` delivery is covered by a regression test.
- A minimal byte-oriented terminal state engine exists for printable bytes,
  CR/LF, backspace, tab, wrapping, cursor movement, erase line/display, OSC
  title, and basic SGR attributes. It is unit-tested but is not yet integrated
  into the daemon as a compatibility surface.
- Interactive attach supports the default `C-a d` detach path for the candidate
  single-window daemon.
- The differential test suite includes cases for detached lifecycle,
  attached create, attach-or-create create behavior, `-ls`/`-wipe` discovery
  output including filtered `-ls [match]`, child environment, `-T` terminal
  override, child-exit cleanup, `-s shell`, compact detached-create options,
  startup config from `-c`, `SCREENRC`, and `$HOME/.screenrc`, config `source`,
  config `chdir`, `-L` default logging, config-driven logging, and
  `-p 0 -X stuff` against the configured GNU Screen reference. Live GNU Screen
  execution is skipped when the environment does not permit Unix socket
  binding.

## Local Reference Notes

The bundled macOS `/usr/bin/screen` observed during development reports GNU
Screen 4.00.03 and does not support `-Q`. Query-command compatibility therefore
remains parser-only locally until a GNU Screen 4.9.x or 5.0.x reference binary
is supplied through `SCREEN_REFERENCE`.
