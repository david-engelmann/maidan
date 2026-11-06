#!/usr/bin/env bash
# Compose privilege gate: every Maidan server service in the checked-in compose
# files must drop all capabilities and set no-new-privileges, and the CLI image
# must run as nonroot. Without this, a compose up of the scale profile once
# shipped maidan-r1/r2 with the container's default capability set.
#
# Usage: scripts/check-compose-security.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import re, sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print("compose security: PyYAML required", file=sys.stderr)
    sys.exit(1)

SERVER_RE = re.compile(r"^(maidan-server|maidan-r[12]|maidan-[ab]|maidan-otlp)$")
errors = []
checked = 0

for path in (Path("compose.yaml"), Path("compose.dev.yaml"), Path("compose.pitr.yaml")):
    if not path.exists():
        continue
    raw = path.read_text()
    docs = [d for d in yaml.safe_load_all(raw) if isinstance(d, dict)]
    for doc in docs:
        services = doc.get("services") or {}
        if not isinstance(services, dict):
            continue
        for name, svc in services.items():
            if not SERVER_RE.match(name):
                continue
            if not isinstance(svc, dict):
                errors.append(f"{path}: service {name} must be a mapping")
                continue
            checked += 1
            opts = svc.get("security_opt") or []
            if not isinstance(opts, list) or not any(
                (isinstance(o, str) and o.replace(" ", "") in ("no-new-privileges:true", "no-new-privileges=true"))
                for o in opts
            ):
                errors.append(f"{path}: {name} must set security_opt no-new-privileges:true")
            caps = svc.get("cap_drop") or []
            if not isinstance(caps, list) or "ALL" not in caps:
                errors.append(f"{path}: {name} must cap_drop ALL")

cli = Path("docker/Dockerfile.cli").read_text()
checked += 1
if not re.search(r"(?m)^USER\s+nonroot(:nonroot)?\s*$", cli):
    errors.append("docker/Dockerfile.cli must USER nonroot:nonroot")

if checked == 0:
    errors.append("compose security: no maidan server services found")

if errors:
    print("compose security:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"compose security: {checked} check(s) ok")
PY
