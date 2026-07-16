#!/usr/bin/env python3
"""Summarize compatibility feature manifest coverage.

By default this prints a Markdown summary for all manifests under
compatibility/features. Use --check <path> in CI to ensure a checked-in report is
in sync with the manifest.
"""

from __future__ import annotations

import argparse
import collections
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
STATUS_ORDER = ["implemented", "partial", "missing", "unsupported"]


def load_manifests(paths: list[pathlib.Path]) -> list[tuple[pathlib.Path, dict]]:
    if not paths:
        paths = sorted((ROOT / "compatibility" / "features").glob("*.toml"))
    manifests = []
    for path in paths:
        manifests.append((path, tomllib.loads(path.read_text())))
    return manifests


def markdown(manifests: list[tuple[pathlib.Path, dict]]) -> str:
    lines: list[str] = []
    lines.append("# Compatibility Manifest Summary")
    lines.append("")
    for path, data in manifests:
        profile = data.get("profile", {})
        features = data.get("features", [])
        status_counts = collections.Counter(feature["status"] for feature in features)
        surface_counts = collections.Counter(feature["surface"] for feature in features)
        missing = [feature for feature in features if feature["status"] == "missing"]

        lines.append(f"## {profile.get('name', path.stem)}")
        lines.append("")
        lines.append(f"- Manifest: `{path.relative_to(ROOT)}`")
        lines.append(f"- Reference: {profile.get('reference', 'unknown')}")
        lines.append(f"- Profile status: {profile.get('status', 'unknown')}")
        lines.append(f"- Total features: {len(features)}")
        lines.append("")
        lines.append("### By status")
        lines.append("")
        lines.append("| Status | Count |")
        lines.append("|---|---:|")
        for status in STATUS_ORDER:
            lines.append(f"| {status} | {status_counts.get(status, 0)} |")
        for status in sorted(set(status_counts) - set(STATUS_ORDER)):
            lines.append(f"| {status} | {status_counts[status]} |")
        lines.append("")
        lines.append("### By surface")
        lines.append("")
        lines.append("| Surface | Count |")
        lines.append("|---|---:|")
        for surface, count in sorted(surface_counts.items()):
            lines.append(f"| {surface} | {count} |")
        lines.append("")
        lines.append("### Missing work items")
        lines.append("")
        if missing:
            lines.append("| ID | Surface | Name | Notes |")
            lines.append("|---|---|---|---|")
            for feature in sorted(missing, key=lambda f: f["id"]):
                notes = feature.get("notes", "").replace("|", "/")
                lines.append(
                    f"| `{feature['id']}` | {feature['surface']} | `{feature['name']}` | {notes} |"
                )
        else:
            lines.append("No missing entries.")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifests", nargs="*", type=pathlib.Path)
    parser.add_argument("--check", type=pathlib.Path, help="compare output with a checked-in file")
    args = parser.parse_args(argv[1:])

    rendered = markdown(load_manifests(args.manifests))
    if args.check:
        expected = args.check.read_text() if args.check.exists() else ""
        if expected != rendered:
            print(f"compatibility summary is stale: {args.check}", file=sys.stderr)
            return 1
        print(f"compatibility summary is current: {args.check}")
        return 0
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
