# Upstream Reference

GNU Screen is the compatibility reference.

Set the reference executable with:

```sh
SCREEN_REFERENCE=/usr/bin/screen cargo test --workspace
```

Future compatibility profiles must record the GNU Screen version, operating
system, architecture, terminal type, shell, locale, and enabled build features.
