#!/usr/bin/env bash
# Every third-party GitHub Action in .github/workflows must be pinned to a
# 40-character commit SHA. The version tag or branch may appear in a trailing
# comment. Local path actions (./...) are allowed; docker:// image refs are
# not used here and would need a digest.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import re
import sys
from pathlib import Path

uses_re = re.compile(r"^(\s*(?:- )?uses:\s*)(\S+)$")
sha_re = re.compile(r"^[0-9a-fA-F]{40}$")
unpinned = []
count = 0
for path in sorted(Path(".github/workflows").glob("*.yml")):
    for i, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.split("#", 1)[0].rstrip()
        if not line or line.lstrip().startswith("#"):
            continue
        m = uses_re.match(line)
        if not m:
            if re.search(r"(^|\s)uses:", line):
                unpinned.append(f"{path}:{i}: could not parse: {raw}")
            continue
        spec = m.group(2).strip("'\"")
        count += 1
        if spec.startswith("./") or spec.startswith(".github/"):
            continue
        if "@" not in spec:
            unpinned.append(f"{path}:{i}: missing ref: {raw}")
            continue
        _action, ref = spec.rsplit("@", 1)
        if not sha_re.fullmatch(ref):
            unpinned.append(f"{path}:{i}: unpinned {spec}")

if unpinned:
    print("check-pinned-actions: unpinned or unparseable uses:", file=sys.stderr)
    for row in unpinned:
        print(f"  {row}", file=sys.stderr)
    sys.exit(1)
print(f"check-pinned-actions: ok ({count} uses: pinned to a commit SHA)")
PY
