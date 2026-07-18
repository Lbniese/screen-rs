# Missing Compatibility Work Items

Source: `compatibility/features/screen-5.0.2.toml`.

These are the manifest entries still marked `missing`. Each item should stay open until implementation and differential or unit coverage are added, then the manifest status can move to `partial` or `implemented`.

- [x] `cli.dashA` (cli_option, `-A`)
  - **DONE**: CLI parsing implemented; marked as `partial` in manifest
  - Remaining: Runtime adapt-all display resize behavior
- [x] `cli.dashO` (cli_option, `-O`)
  - **DONE**: CLI parsing implemented; marked as `partial` in manifest
  - Remaining: Runtime optimal-output emulation mode
- [x] `cli.dashU` (cli_option, `-U`)
  - **DONE**: CLI parsing implemented; marked as `partial` in manifest
  - Remaining: Runtime UTF-8 mode behavior
- [x] `cli.dasha` (cli_option, `-a`)
  - **DONE**: CLI parsing implemented; marked as `partial` in manifest
  - Remaining: Runtime termcap force-all behavior
- [x] `cli.dashf` (cli_option, `-f`, `-fn`, `-fa`)
  - **DONE**: CLI parsing implemented for all flow control variants; marked as `partial` in manifest
  - Remaining: Runtime flow control behavior
- [x] `cli.dashi` (cli_option, `-i`)
  - **DONE**: CLI parsing implemented; marked as `partial` in manifest
  - Remaining: Runtime interrupt-output-sooner behavior
- [x] `cli.dashq` (cli_option, `-q`)
  - **DONE**: CLI parsing implemented; marked as `partial` in manifest
  - Remaining: Runtime quiet startup behavior
- [x] `cli.dashx` (cli_option, `-x`)
  - **COMPLETE**: Full multi-display attach mode implemented
  - Daemon detaches existing clients on normal attach
  - Daemon allows multiple simultaneous attaches with -x flag
  - Protocol updated to support multi_display flag in Attach message
- [ ] `interactive.copy_mode_keys` (interactive_key, `copy mode keys`)
  - Gap: Copy-mode navigation and selection key parity is not complete.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `platform.utmp` (platform, `utmp/utmpx login accounting`)
  - Gap: Platform accounting is documented as future work and not implemented to parity.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `runtime.multi_display` (runtime, `multiple displays`)
  - Gap: GNU -x/multiple simultaneous display semantics are not implemented to parity.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
