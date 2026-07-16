#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <screen-version> [prefix]" >&2
  echo "example: $0 4.9.1 .local/screen-4.9.1" >&2
  exit 64
fi

version="$1"
prefix="${2:-.local/screen-$version}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$repo_root/.local" "$prefix"

if [[ -x "$prefix/bin/screen" ]]; then
  echo "existing install found at $prefix/bin/screen" >&2
  "$prefix/bin/screen" --version
  exit 0
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/screen-build.${version}.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT
archive="screen-$version.tar.gz"
url="https://ftp.gnu.org/gnu/screen/$archive"

cd "$workdir"
echo "downloading $url" >&2
curl -L --fail --show-error -o "$archive" "$url"
tar -xzf "$archive"
cd "screen-$version"

jobs="${BUILD_JOBS:-2}"
echo "configuring screen-$version -> $prefix" >&2
./configure --prefix="$(cd "$repo_root" && mkdir -p "$prefix" && cd "$prefix" && pwd)"
echo "building screen-$version with -j$jobs" >&2
make -j"$jobs"
echo "installing screen-$version" >&2
make install

echo "installed: $prefix/bin/screen" >&2
"$prefix/bin/screen" --version
