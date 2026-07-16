#!/usr/bin/env python3
"""Validate screen-rs compatibility feature manifests.

The manifest is intentionally simple TOML so it can be checked in CI without
adding Rust dependencies.  Policy enforced here:

* each feature has a stable id, surface, status, and notes;
* statuses are one of missing/partial/implemented/unsupported;
* implemented features must name at least one unit/integration/differential test;
* partial, missing, and unsupported features must explain the gap in notes;
* all named test files must exist.
"""

from __future__ import annotations

import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
VALID_STATUSES = {"missing", "partial", "implemented", "unsupported"}
TEST_KEYS = ("unit_tests", "integration_tests", "differential_tests")


def fail(message: str) -> None:
    print(f"compatibility manifest error: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate_file(path: pathlib.Path) -> None:
    data = tomllib.loads(path.read_text())
    features = data.get("features")
    if not isinstance(features, list) or not features:
        fail(f"{path}: expected non-empty [[features]] array")

    seen: set[str] = set()
    for index, feature in enumerate(features, start=1):
        if not isinstance(feature, dict):
            fail(f"{path}: feature #{index} is not a table")
        feature_id = feature.get("id")
        if not isinstance(feature_id, str) or not feature_id.strip():
            fail(f"{path}: feature #{index} missing id")
        if feature_id in seen:
            fail(f"{path}: duplicate feature id {feature_id}")
        seen.add(feature_id)

        surface = feature.get("surface")
        if not isinstance(surface, str) or not surface.strip():
            fail(f"{path}: {feature_id}: missing surface")

        status = feature.get("status")
        if status not in VALID_STATUSES:
            fail(f"{path}: {feature_id}: invalid status {status!r}")

        notes = feature.get("notes")
        if not isinstance(notes, str) or not notes.strip():
            fail(f"{path}: {feature_id}: notes are required")

        tests: list[str] = []
        for key in TEST_KEYS:
            values = feature.get(key, [])
            if values is None:
                values = []
            if not isinstance(values, list) or not all(isinstance(v, str) for v in values):
                fail(f"{path}: {feature_id}: {key} must be an array of strings")
            tests.extend(values)

        if status == "implemented" and not tests:
            fail(f"{path}: {feature_id}: implemented feature lacks tests")

        for test in tests:
            test_path = ROOT / test
            if not test_path.exists():
                fail(f"{path}: {feature_id}: referenced test path does not exist: {test}")


def main(argv: list[str]) -> int:
    manifest_paths = [pathlib.Path(arg) for arg in argv[1:]]
    if not manifest_paths:
        manifest_paths = sorted((ROOT / "compatibility" / "features").glob("*.toml"))
    if not manifest_paths:
        fail("no compatibility feature manifests found")
    for manifest in manifest_paths:
        validate_file(manifest)
    print(f"validated {len(manifest_paths)} compatibility manifest(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
