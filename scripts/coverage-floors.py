#!/usr/bin/env python3
"""Fail when line coverage drops below its floor.

Reads an lcov report (``cargo llvm-cov report --lcov``) and the floors in
``.config/coverage-floors.toml``, totals covered and instrumented lines per
workspace crate, and exits non-zero when the workspace or any crate is under
its floor. A crate that shows up in the report without a floor, or a floor for
a crate the report does not cover, is also a failure: a new crate gets a floor
when it lands, and a removed one takes its floor with it.

Usage: coverage-floors.py LCOV [--floors PATH]
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

CRATE_PATH = re.compile(r"(?:^|/)crates/([^/]+)/")


@dataclass
class Lines:
    covered: int = 0
    found: int = 0

    @property
    def percent(self) -> float:
        return 100.0 * self.covered / self.found if self.found else 100.0


def per_crate(lcov: str) -> dict[str, Lines]:
    """Covered and instrumented lines per crate, from lcov ``SF``/``LH``/``LF``."""
    totals: dict[str, Lines] = {}
    crate: str | None = None
    for raw in lcov.splitlines():
        line = raw.strip()
        if line.startswith("SF:"):
            match = CRATE_PATH.search(line[3:])
            crate = match.group(1) if match else None
        elif crate is None:
            continue
        elif line.startswith("LF:"):
            totals.setdefault(crate, Lines()).found += int(line[3:])
        elif line.startswith("LH:"):
            totals.setdefault(crate, Lines()).covered += int(line[3:])
        elif line == "end_of_record":
            crate = None
    return totals


def check(totals: dict[str, Lines], floors: dict) -> tuple[list[str], list[str]]:
    """Report rows and failures for ``totals`` against ``floors``."""
    crate_floors: dict[str, float] = floors.get("crates", {})
    workspace_floor = float(floors["workspace"]["lines"])
    rows = [f"{'crate':<24} {'lines':>15} {'covered':>8} {'floor':>6}"]
    failures: list[str] = []

    for crate in sorted(totals.keys() | crate_floors.keys()):
        lines = totals.get(crate)
        floor = crate_floors.get(crate)
        if lines is None:
            failures.append(f"{crate}: has a floor but no coverage data; remove its floor")
            continue
        shown = f"{lines.covered}/{lines.found}"
        if floor is None:
            rows.append(f"{crate:<24} {shown:>15} {lines.percent:7.2f}% {'-':>6}")
            failures.append(f"{crate}: {lines.percent:.2f}% of lines covered, but no floor is set")
            continue
        rows.append(f"{crate:<24} {shown:>15} {lines.percent:7.2f}% {float(floor):6.1f}")
        if lines.percent < floor:
            failures.append(f"{crate}: {lines.percent:.2f}% of lines covered, under its {floor}% floor")

    whole = Lines(
        covered=sum(l.covered for l in totals.values()),
        found=sum(l.found for l in totals.values()),
    )
    shown = f"{whole.covered}/{whole.found}"
    rows.append(f"{'workspace':<24} {shown:>15} {whole.percent:7.2f}% {workspace_floor:6.1f}")
    if whole.percent < workspace_floor:
        failures.append(
            f"workspace: {whole.percent:.2f}% of lines covered, under its {workspace_floor}% floor"
        )
    return rows, failures


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("lcov", type=Path)
    parser.add_argument("--floors", type=Path, default=Path(".config/coverage-floors.toml"))
    args = parser.parse_args(argv)

    totals = per_crate(args.lcov.read_text(encoding="utf-8"))
    if not totals:
        print(f"no workspace crates in {args.lcov}", file=sys.stderr)
        return 1
    with args.floors.open("rb") as source:
        floors = tomllib.load(source)
    rows, failures = check(totals, floors)

    table = "\n".join(rows)
    print(table)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as out:
            out.write(f"### Line coverage against floors\n\n```\n{table}\n```\n")
    for failure in failures:
        print(f"::error::{failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
