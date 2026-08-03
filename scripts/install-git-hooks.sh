#!/usr/bin/env bash
# Install screen-rs git hooks into .git/hooks (idempotent).
#
# Hooks live under scripts/git-hooks so they are versioned with the repo;
# this script symlinks them into the local .git/hooks directory. Re-run it
# after a fresh clone.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
hooks_dir="$root/.git/hooks"

if [[ ! -d "$hooks_dir" ]]; then
  echo "error: $hooks_dir not found; run this from inside a screen-rs clone" >&2
  exit 1
fi

mkdir -p "$hooks_dir"

for hook in pre-push; do
  src="$root/scripts/git-hooks/$hook"
  dst="$hooks_dir/$hook"

  chmod +x "$src"
  # Replace any existing file/symlink with a fresh symlink to the tracked hook.
  ln -sf "$src" "$dst"
  echo "installed $hook -> $dst"
done

cat <<'EOF'

Git hooks installed. They are symlinks to scripts/git-hooks/, so updates are
picked up automatically.

To bypass on a one-off push:  SCREEN_RS_SKIP_HOOK=1 git push
EOF
