#!/usr/bin/env bash
# No file in the public tree names an internal project, team or repository.
#
# The names are kept as digests in scripts/internal-names.sha256 (a length and
# the SHA-256 of the lowercased name), so the list does not name them either.
# Every tracked file is split into lowercase runs of letters and digits; a run
# that is a listed name, or starts or ends with one (a name run into a
# camelCase identifier, or behind a prefix), fails the check with its file and
# line.
set -euo pipefail
cd "$(dirname "$0")/.."

list="scripts/internal-names.sha256"
[[ -f "$list" ]] || { echo "internal names: $list is missing" >&2; exit 1; }

# A check that fails open is worse than none: an unreadable list, an empty
# one, or a failed file listing refuses to pass.
scan=$(cat <<'PY'
import hashlib
import re
import sys

listed = {}
with open(sys.argv[1], encoding="utf-8") as fh:
    for raw in fh:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        length, digest = line.split()
        listed.setdefault(int(length), set()).add(digest)
if not listed:
    print(f"internal names: {sys.argv[1]} names nothing", file=sys.stderr)
    sys.exit(1)

paths = [p for p in sys.stdin.buffer.read().decode("utf-8").split("\0") if p]
if not paths:
    print("internal names: git ls-files listed nothing; refusing to pass", file=sys.stderr)
    sys.exit(1)

def digest(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()

def names_in(token):
    for length, digests in listed.items():
        if len(token) < length:
            continue
        for part in {token[:length], token[-length:]}:
            if digest(part) in digests:
                return True
    return False

seen = {}
hits = []
for path in paths:
    if path == sys.argv[1]:
        continue
    try:
        data = open(path, "rb").read()
    except (IsADirectoryError, FileNotFoundError):
        continue  # a submodule or a file deleted in the work tree
    if b"\0" in data[:8192]:
        continue  # binary
    text = data.decode("utf-8", errors="ignore").lower()
    for number, line in enumerate(text.splitlines(), 1):
        for token in re.findall(r"[a-z0-9]+", line):
            found = seen.get(token)
            if found is None:
                found = seen[token] = names_in(token)
            if found:
                hits.append(f"{path}:{number}")
                break

if hits:
    print("internal names: the public tree names an internal project again:", file=sys.stderr)
    for hit in hits:
        print(f"  {hit}", file=sys.stderr)
    print("internal names: use a neutral name, or cite the contract by its title", file=sys.stderr)
    sys.exit(1)
count = sum(len(d) for d in listed.values())
print(f"internal names: none of the {count} listed names appear in {len(paths)} files")
PY
)
git ls-files -z | python3 -c "$scan" "$list"
