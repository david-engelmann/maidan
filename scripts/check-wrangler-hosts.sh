#!/usr/bin/env bash
# Every Cloudflare wrangler config must pin a real maidan.world custom domain,
# keep workers.dev off, and (when set) make MAIDAN_PUBLIC_ORIGIN match that
# https origin. A workers.dev=true or mismatched origin once shipped the edge
# Worker to a stranger-controlled subdomain with the production bindings.
#
# Usage: scripts/check-wrangler-hosts.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, re, sys
from pathlib import Path

root = Path("deploy/cloudflare")
configs = sorted(root.glob("wrangler*.jsonc"))
if not configs:
    print("wrangler hosts: no deploy/cloudflare/wrangler*.jsonc found", file=sys.stderr)
    sys.exit(1)

# Strip // line comments and /* */ blocks; keep strings intact enough for our keys.
def loads_jsonc(text: str):
    out = []
    i, n = 0, len(text)
    in_str = False
    esc = False
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

host_re = re.compile(r"^(?:[a-z0-9-]+\.)*maidan\.world$")
errors = []
checked = 0
for path in configs:
    try:
        doc = loads_jsonc(path.read_text())
    except Exception as e:
        errors.append(f"{path}: invalid JSONC ({e})")
        continue
    checked += 1
    if doc.get("workers_dev") is not False:
        errors.append(f"{path}: workers_dev must be false (got {doc.get('workers_dev')!r})")
    routes = doc.get("routes") or []
    if not isinstance(routes, list) or not routes:
        errors.append(f"{path}: routes must list at least one custom_domain")
        continue
    hosts = []
    for idx, route in enumerate(routes):
        if not isinstance(route, dict):
            errors.append(f"{path}: routes[{idx}] must be an object")
            continue
        if route.get("custom_domain") is not True:
            errors.append(f"{path}: routes[{idx}] must set custom_domain: true")
        pattern = route.get("pattern")
        if not isinstance(pattern, str) or not host_re.fullmatch(pattern):
            errors.append(f"{path}: routes[{idx}].pattern must be a maidan.world host (got {pattern!r})")
        else:
            hosts.append(pattern)
    vars_ = doc.get("vars") or {}
    origin = vars_.get("MAIDAN_PUBLIC_ORIGIN")
    if origin is not None:
        if not isinstance(origin, str) or not origin.startswith("https://"):
            errors.append(f"{path}: MAIDAN_PUBLIC_ORIGIN must be https (got {origin!r})")
        else:
            host = origin[len("https://"):].split("/", 1)[0].lower()
            if ":" in host or "@" in host:
                errors.append(f"{path}: MAIDAN_PUBLIC_ORIGIN must not carry userinfo or a port")
            elif host not in hosts:
                errors.append(
                    f"{path}: MAIDAN_PUBLIC_ORIGIN host {host!r} must match a routes[].pattern {hosts}"
                )

if errors:
    print("wrangler hosts:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"wrangler hosts: {checked} config(s) ok")
PY
