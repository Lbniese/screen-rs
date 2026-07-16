# Compatibility Manifest Summary

## linux-glibc-screen-5.0.2

- Manifest: `compatibility/features/screen-5.0.2.toml`
- Reference: GNU Screen 5.0.2
- Profile status: pre-claim
- Total features: 359

### By status

| Status | Count |
|---|---:|
| implemented | 77 |
| partial | 271 |
| missing | 11 |
| unsupported | 0 |

### By surface

| Surface | Count |
|---|---:|
| cli_option | 34 |
| config_or_colon_command | 185 |
| interactive_key | 10 |
| platform | 1 |
| query_command | 14 |
| remote_command | 100 |
| runtime | 5 |
| terminal | 10 |

### Missing work items

| ID | Surface | Name | Notes |
|---|---|---|---|
| `cli.dashA` | cli_option | `-A` | Adapt-all display resize behavior is not implemented yet. |
| `cli.dashO` | cli_option | `-O` | Optimal-output emulation mode is not implemented. |
| `cli.dashU` | cli_option | `-U` | CLI UTF-8 mode switch is not claimed. |
| `cli.dasha` | cli_option | `-a` | Parsed/runtime termcap force-all behavior is not implemented yet. |
| `cli.dashf` | cli_option | `-f` | CLI flow-control option behavior is not implemented as a parity surface. |
| `cli.dashi` | cli_option | `-i` | Interrupt-output-sooner behavior is not implemented. |
| `cli.dashq` | cli_option | `-q` | Quiet startup semantics are not implemented. |
| `cli.dashx` | cli_option | `-x` | Multi-display attach mode is not implemented to GNU parity. |
| `interactive.copy_mode_keys` | interactive_key | `copy mode keys` | Copy-mode navigation and selection key parity is not complete. |
| `platform.utmp` | platform | `utmp/utmpx login accounting` | Platform accounting is documented as future work and not implemented to parity. |
| `runtime.multi_display` | runtime | `multiple displays` | GNU -x/multiple simultaneous display semantics are not implemented to parity. |
