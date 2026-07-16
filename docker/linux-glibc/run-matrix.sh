#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
image="screen-rs-linux-glibc"

docker build -t "$image" -f "$repo_root/docker/linux-glibc/Dockerfile" "$repo_root"

docker run --rm \
  -v "$repo_root:/workspace" \
  -w /workspace \
  -e CARGO_TARGET_DIR=/workspace/target-linux-glibc \
  -e SCREEN_CANDIDATE=/workspace/target-linux-glibc/debug/screen-rs \
  -e SCREEN_REFERENCE_ROOT=/workspace/.local/linux-glibc \
  "$image" \
  bash -c './scripts/run-differential-matrix.sh 4.9.1 5.0.2'
