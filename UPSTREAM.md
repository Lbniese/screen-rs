# Upstream Reference

GNU Screen is the compatibility reference.

## Supported reference installs

Build exact reference versions locally:

```sh
./scripts/install-screen-reference.sh 4.9.1
./scripts/install-screen-reference.sh 5.0.2
```

Tarballs:

- https://ftp.gnu.org/gnu/screen/screen-4.9.1.tar.gz
- https://ftp.gnu.org/gnu/screen/screen-5.0.2.tar.gz

Resulting binaries:

- `.local/screen-4.9.1/bin/screen`
- `.local/screen-5.0.2/bin/screen`

## Running against one reference

```sh
SCREEN_REFERENCE=.local/screen-4.9.1/bin/screen cargo test --workspace
```

## Running the differential matrix

```sh
./scripts/run-differential-matrix.sh 4.9.1 5.0.2
```

## Running the Linux/glibc matrix container

```sh
./docker/linux-glibc/run-matrix.sh
```

Profiles live in:

- `compatibility/profiles/linux-glibc-screen-4.9.1.toml`
- `compatibility/profiles/linux-glibc-screen-5.0.2.toml`

Future compatibility profiles must record the GNU Screen version, operating
system, architecture, terminal type, shell, locale, and enabled build features.
