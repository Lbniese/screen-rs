# Missing Compatibility Work Items

Source: `compatibility/features/screen-5.0.2.toml`.

These are the manifest entries still marked `missing`. Each item should stay open until implementation and differential or unit coverage are added, then the manifest status can move to `partial` or `implemented`.

- [ ] `cli.dashA` (cli_option, `-A`)
  - Gap: Adapt-all display resize behavior is not implemented yet.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dashO` (cli_option, `-O`)
  - Gap: Optimal-output emulation mode is not implemented.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dashU` (cli_option, `-U`)
  - Gap: CLI UTF-8 mode switch is not claimed.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dasha` (cli_option, `-a`)
  - Gap: Parsed/runtime termcap force-all behavior is not implemented yet.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dashf` (cli_option, `-f`)
  - Gap: CLI flow-control option behavior is not implemented as a parity surface.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dashi` (cli_option, `-i`)
  - Gap: Interrupt-output-sooner behavior is not implemented.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dashq` (cli_option, `-q`)
  - Gap: Quiet startup semantics are not implemented.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `cli.dashx` (cli_option, `-x`)
  - Gap: Multi-display attach mode is not implemented to GNU parity.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `interactive.copy_mode_keys` (interactive_key, `copy mode keys`)
  - Gap: Copy-mode navigation and selection key parity is not complete.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `platform.utmp` (platform, `utmp/utmpx login accounting`)
  - Gap: Platform accounting is documented as future work and not implemented to parity.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
- [ ] `runtime.multi_display` (runtime, `multiple displays`)
  - Gap: GNU -x/multiple simultaneous display semantics are not implemented to parity.
  - Done when: behavior is implemented or explicitly classified as unsupported with rationale, and manifest/test references are updated.
