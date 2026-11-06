#!/usr/bin/env bash
# Cloudflare wrangler fleet cost + security gates for every deploy/cloudflare/wrangler*.jsonc.
# - workers_dev must be false (no *.workers.dev exposure)
# - container workers: max_instances=1, instance_type=basic, cron ≤1×/day when present
# - docs worker: static assets only (no main/containers/triggers)
# - vars must not embed secret-shaped keys; routes must be *.maidan.world custom domains
#
# Usage: scripts/check-wrangler-fleet-gates.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(".")
files = sorted(ROOT.glob("deploy/cloudflare/wrangler*.jsonc"))
if not files:
    print("wrangler-fleet gates: no wrangler*.jsonc found", file=sys.stderr)
    sys.exit(1)

secret_key = re.compile(
    r'"(?P<k>[A-Z0-9_]*(?:SECRET|PASSWORD|PRIVATE_KEY|API_KEY|CONTENT_KEK|SESSION_SECRET|DATABASE_URL)[A-Z0-9_]*)"\s*:'
)
cron_re = re.compile(r'"crons"\s*:\s*\[\s*"([^"]+)"\s*\]')
route_re = re.compile(
    r'\{\s*"pattern"\s*:\s*"([^"]+)"\s*,\s*"custom_domain"\s*:\s*(true|false)\s*\}'
)
checked = 0


def require(cond: bool, msg: str) -> None:
    global checked
    checked += 1
    if not cond:
        print(f"wrangler-fleet gates: {msg}", file=sys.stderr)
        sys.exit(1)


for path in files:
    rel = path.as_posix()
    text = path.read_text(encoding="utf-8")
    # Strip // line comments for structural scans (jsonc).
    stripped = "\n".join(
        line if not line.lstrip().startswith("//") else ""
        for line in text.splitlines()
    )

    require('"workers_dev": false' in stripped or '"workers_dev":false' in stripped,
            f"{rel} must set workers_dev: false")

    for m in secret_key.finditer(stripped):
        require(False, f"{rel} vars must not embed secret-shaped key {m.group('k')}")

    for m in route_re.finditer(stripped):
        pattern, custom = m.group(1), m.group(2)
        require(pattern.endswith("maidan.world") or pattern.endswith(".maidan.world"),
                f"{rel} route {pattern!r} must be a maidan.world host")
        require(custom == "true", f"{rel} route {pattern!r} must use custom_domain: true")

    is_docs = path.name == "wrangler.docs.jsonc"
    if is_docs:
        require('"main"' not in stripped, f"{rel} must stay static (no main)")
        require('"containers"' not in stripped, f"{rel} must stay static (no containers)")
        require('"triggers"' not in stripped, f"{rel} must stay static (no triggers)")
        require('"assets"' in stripped, f"{rel} must declare assets")
        continue

    require('"containers"' in stripped, f"{rel} must declare containers")
    require(re.search(r'"max_instances"\s*:\s*1\b', stripped) is not None,
            f"{rel} must pin max_instances: 1")
    require(re.search(r'"instance_type"\s*:\s*"basic"', stripped) is not None,
            f"{rel} must pin instance_type: basic")

    m = cron_re.search(stripped)
    require(m is not None, f"{rel} must declare a triggers.crons schedule")
    expr = m.group(1)
    parts = expr.split()
    require(len(parts) == 5, f"{rel} cron must be a 5-field expression, got {expr!r}")
    minute, hour, dom, month, dow = parts
    require(hour != "*", f"{rel} cron must not wake more than once per hour ({expr})")
    require(dom == "*" and month == "*", f"{rel} cron should run daily (dom/month *), got {expr}")
    require(dow == "*", f"{rel} cron must not restrict to weekdays only ({expr})")
    require(expr != "0 * * * *", f"{rel} must not use the hourly wake cron")
    # Minute and hour fixed => ≤1×/day.
    require(minute != "*" and hour != "*", f"{rel} cron must wake at most once per day ({expr})")

print(f"wrangler-fleet gates: {checked} check(s) ok")
PY
