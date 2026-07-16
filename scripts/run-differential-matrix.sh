#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

candidate="${SCREEN_CANDIDATE:-$repo_root/target/debug/screen-rs}"
reference_root="${SCREEN_REFERENCE_ROOT:-$repo_root/.local}"
versions=("${@:-4.9.1 5.0.2}")
# shellcheck disable=SC2206
versions=(${versions[*]})

tests=(
  differential_cli
  differential_session
  differential_x_commands
  differential_fullscreen
)

mkdir -p compatibility/reports/raw compatibility/reports

if [[ ! -x "$candidate" ]]; then
  cargo build --workspace
fi

summary="compatibility/reports/current-matrix.md"
{
  echo "# Differential Matrix"
  echo
  echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
  echo "Host: $(uname -srm)"
  echo
  echo "Candidate: $candidate"
  echo
  echo "| Reference | Suite | Result | Notes |"
  echo "|---|---|---|---|"
} > "$summary"

for version in "${versions[@]}"; do
  prefix="$reference_root/screen-$version"
  if [[ ! -x "$prefix/bin/screen" ]]; then
    "$repo_root/scripts/install-screen-reference.sh" "$version" "$prefix"
  fi
  reference="$prefix/bin/screen"

  for test_name in "${tests[@]}"; do
    raw="compatibility/reports/raw/${test_name}-screen-${version}.txt"
    set +e
    SCREEN_REFERENCE="$reference" SCREEN_CANDIDATE="$candidate" \
      cargo test -p screen-cli --test "$test_name" -- --nocapture \
      >"$raw" 2>&1
    status=$?
    set -e

    result="PASS"
    notes="all assertions passed"
    if (( status != 0 )); then
      result="FAIL"
      if grep -q "FAILED" "$raw"; then
        notes="$(grep -E 'FAILED|panicked at|assertion `left == right` failed' "$raw" | head -n 3 | tr '|' '/' | tr '\n' '; ' | sed 's/; $//')"
      else
        notes="cargo test exited with status $status"
      fi
    elif grep -q 'result: mismatch' "$raw"; then
      result="MISMATCH"
      notes="structured differential mismatches were reported"
    elif grep -q "skipping" "$raw"; then
      result="PASS/SKIP"
      notes="contains platform or PTY-related skips"
    fi

    printf '| %s | %s | %s | %s |\n' "$version" "$test_name" "$result" "$notes" >> "$summary"
  done

done

echo >> "$summary"
echo '## Common CLI mismatches' >> "$summary"
echo >> "$summary"
for version in "${versions[@]}"; do
  raw="compatibility/reports/raw/differential_cli-screen-${version}.txt"
  echo "### screen $version" >> "$summary"
  if [[ -f "$raw" ]]; then
    if grep -q 'result: mismatch' "$raw"; then
      grep -E 'case:|result: mismatch|- field:' "$raw" >> "$summary" || true
    else
      echo "No CLI mismatches detected." >> "$summary"
    fi
  else
    echo "No CLI log captured." >> "$summary"
  fi
  echo >> "$summary"
done

echo "wrote $summary"
