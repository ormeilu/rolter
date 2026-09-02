#!/usr/bin/env python3
"""Keep charts/rolter/Chart.yaml's appVersion equal to the workspace version.

`appVersion` is what an operator reads to know which rolter a chart release
deploys, and what `helm list` and most dashboards surface — a stale value points
at a version that was never deployed. Nothing updated it, so it silently drifted
three releases behind (#1140).

The chart's own `version:` is a separate number on its own cadence and is never
touched here.

    scripts/sync-chart-appversion.py           # fail if they disagree
    scripts/sync-chart-appversion.py --fix     # rewrite Chart.yaml to match

Deliberately a line rewrite rather than a YAML round-trip: loading and dumping
Chart.yaml would reorder keys and drop comments, and the file is hand-edited.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO = ROOT / "Cargo.toml"
CHART = ROOT / "charts" / "rolter" / "Chart.yaml"

# the version under [workspace.package], not any dependency's
WORKSPACE_VERSION = re.compile(
    r"^\[workspace\.package\]$.*?^version\s*=\s*\"([^\"]+)\"",
    re.MULTILINE | re.DOTALL,
)
APP_VERSION = re.compile(r"^(appVersion:\s*)(\S+)\s*$", re.MULTILINE)


def workspace_version() -> str:
    match = WORKSPACE_VERSION.search(CARGO.read_text())
    if not match:
        sys.exit("::error::could not find [workspace.package] version in Cargo.toml")
    return match.group(1)


def main() -> int:
    fix = "--fix" in sys.argv[1:]
    want = workspace_version()
    chart = CHART.read_text()

    match = APP_VERSION.search(chart)
    if not match:
        sys.exit(f"::error::{CHART.relative_to(ROOT)} has no appVersion key")

    have = match.group(2).strip("\"'")
    if have == want:
        print(f"appVersion {have} matches the workspace version")
        return 0

    if not fix:
        print(
            f"::error::chart appVersion is {have} but the workspace ships {want}. "
            "run scripts/sync-chart-appversion.py --fix"
        )
        return 1

    CHART.write_text(APP_VERSION.sub(rf'\g<1>"{want}"', chart, count=1))
    print(f"appVersion {have} -> {want}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
