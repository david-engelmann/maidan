#!/usr/bin/env bash
# David's hosted cost rule, as a gate: nothing may wake a Cloudflare Container
# (or any other always-billable edge workload) on a timer more than once a day.
# Workflow schedules that only burn GitHub Actions minutes are held to the same
# ceiling so a typo cannot slip an hourly cron into nightly/deploy automation.
#
# Checks:
#   - every `on.schedule[].cron` in .github/workflows/*.yml
#   - every `triggers.crons[]` in deploy/cloudflare/wrangler*.jsonc
#
# A cron is "more than once a day" when its minute or hour field is a step
# (*/N), a list, or a range that expands to multiple firings per day while
# day-of-month and day-of-week are both wildcard.
#
# Usage: scripts/check-ci-cost.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, re, sys
from pathlib import Path

def loads_jsonc(text: str):
    out = []
    i, n = 0, len(text)
    in_str = esc = False
    while i < n:
        ch = text[i]
        if in_str:
            out.append(ch)
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                in_str = False
            i += 1
            continue
        if ch == '"':
            in_str = True
            out.append(ch)
            i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            i += 2
            while i < n and text[i] not in "\n\r":
                i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                i += 1
            i = min(i + 2, n)
            continue
        out.append(ch)
        i += 1
    return json.loads("".join(out))

def field_expansions(field: str) -> int | None:
    """Return how many distinct values a cron field expands to, or None if unknown/safe single."""
    field = field.strip()
    if field == "*":
        return None  # wildcard — constrained by other fields
    if re.fullmatch(r"\d+", field):
        return 1
    if "," in field:
        parts = field.split(",")
        if all(re.fullmatch(r"\d+", p) for p in parts):
            return len(set(parts))
        return 99
    if "-" in field and "/" not in field:
        a, b = field.split("-", 1)
        if re.fullmatch(r"\d+", a) and re.fullmatch(r"\d+", b):
            return max(0, int(b) - int(a) + 1)
        return 99
    m = re.fullmatch(r"\*/(\d+)", field)
    if m:
        step = int(m.group(1))
        if step <= 0:
            return 99
        # minute: 60/step, hour: 24/step — caller interprets
        return step  # sentinel via negative? return special
    m = re.fullmatch(r"(\d+)-(\d+)/(\d+)", field)
    if m:
        return 99
    return 99

def fires_more_than_daily(cron: str) -> str | None:
    parts = cron.split()
    if len(parts) != 5:
        return f"expected 5-field cron, got {cron!r}"
    minute, hour, dom, mon, dow = parts
    # Any step/list/range on minute with wild day fields => many/day
    def is_multi(field, modulus):
        field = field.strip()
        if field == "*":
            return False
        if re.fullmatch(r"\d+", field):
            return False
        if re.fullmatch(r"\*/(\d+)", field):
            step = int(field.split("/")[1])
            # Any step > 1 fires more than once per day/hour cycle
            # (e.g. hour */13 => 00 and 13; minute */45 => :00 and :45).
            # Floor division undercounted those; refuse every step greater than 1.
            return (not step) or step > 1
        if "," in field or "-" in field:
            return True
        return True

    # Weekly/monthly constraints (dom or dow not *) can still be >1/day if hour/minute multi
    daily_unconstrained = (dom == "*" and dow == "*")
    if is_multi(minute, 60):
        return "minute field fires more than once per hour"
    if is_multi(hour, 24):
        return "hour field fires more than once per day"
    # minute and hour are single values (or *) 
    # If hour is *, every hour => >1/day unless dow/dom restrict to never? hour * with minute 0 = 24/day
    if hour == "*":
        return "hour=* fires every hour"
    if minute == "*" and hour != "*":
        # every minute of that hour = 60/day
        return "minute=* fires every minute of the hour"
    # Single minute + single hour: at most 1/day when dom/dow wild, or 1 on matching days.
    return None

errors = []
checked = 0

# Workflows
try:
    import yaml
except ImportError:
    print("ci cost: PyYAML is required to parse workflow schedules", file=sys.stderr)
    sys.exit(1)

for path in sorted(Path(".github/workflows").glob("*.yml")):
    doc = yaml.safe_load(path.read_text()) or {}
    on = doc.get(True) or doc.get("on") or {}
    if not isinstance(on, dict):
        continue
    schedules = on.get("schedule") or []
    if not isinstance(schedules, list):
        continue
    for entry in schedules:
        if not isinstance(entry, dict):
            continue
        cron = entry.get("cron")
        if not isinstance(cron, str):
            continue
        checked += 1
        why = fires_more_than_daily(cron)
        if why:
            errors.append(f"{path}: schedule cron {cron!r} — {why}")

# Wrangler crons (wake containers)
for path in sorted(Path("deploy/cloudflare").glob("wrangler*.jsonc")):
    try:
        doc = loads_jsonc(path.read_text())
    except Exception as e:
        errors.append(f"{path}: invalid JSONC ({e})")
        continue
    crons = ((doc.get("triggers") or {}) if isinstance(doc.get("triggers"), dict) else {}).get("crons") or []
    if not isinstance(crons, list):
        continue
    for cron in crons:
        if not isinstance(cron, str):
            continue
        checked += 1
        why = fires_more_than_daily(cron)
        if why:
            errors.append(f"{path}: triggers.crons {cron!r} — {why} (wakes a container)")

if checked == 0:
    errors.append("ci cost: found no schedules or wrangler crons to check")

if errors:
    print("ci cost:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"ci cost: {checked} cron(s) at most once per day — ok")
PY
