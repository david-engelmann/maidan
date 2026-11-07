#!/usr/bin/env bash
# No file in the public tree names an internal project, team, repository,
# agent or deployment.
#
# The names are kept as digests (a length and the SHA-256 of the lowercased
# name), so the lists do not name them either. There are two lists, because
# some names are distinctive anywhere and others are ordinary words:
#
# - scripts/internal-names.sha256, substrings. Every tracked file is split into
#   lowercase runs of letters and digits; a run that is a listed name, or
#   contains one anywhere inside it (a name run into a camelCase identifier,
#   behind a prefix, or between other letters), fails.
# - scripts/internal-tokens.sha256, whole tokens. Every tracked file except a
#   lockfile is split into lowercase runs of letters, digits, `.`, `-` and `/`,
#   trimmed of those three at either end. A listed token fails when it is a
#   whole run, a whole `/`-separated segment of one or two adjacent segments
#   (so `owner/repo` matches inside a URL), or a whole word between `.`, `-`
#   and `/`. A token never matches inside a longer word, so a short name that
#   is also part of ordinary words can be listed here. Lockfiles are skipped
#   because their hashes and package names are not prose.
#
# A hit prints its file and line and fails. To add a name to either list:
#   printf %s 'name' | tr '[:upper:]' '[:lower:]' | shasum -a 256
# and prepend its length. Don't list a word with a common meaning (one the
# product or its docs use on their own), or the check fails on ordinary prose.
set -euo pipefail
cd "$(dirname "$0")/.."

names="scripts/internal-names.sha256"
tokens="scripts/internal-tokens.sha256"
for list in "$names" "$tokens"; do
  [[ -f "$list" ]] || { echo "internal names: $list is missing" >&2; exit 1; }
done

# A check that fails open is worse than none: an unreadable list, an empty
# one, or a failed file listing refuses to pass.
scan=$(cat <<'PY'
import hashlib
import re
import sys

def load(path):
    listed = {}
    with open(path, encoding="utf-8") as fh:
        for raw in fh:
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            length, digest = line.split()
            listed.setdefault(int(length), set()).add(digest)
    if not listed:
        print(f"internal names: {path} names nothing", file=sys.stderr)
        sys.exit(1)
    return listed

names_path, tokens_path = sys.argv[1], sys.argv[2]
names = load(names_path)
tokens = load(tokens_path)

paths = [p for p in sys.stdin.buffer.read().decode("utf-8").split("\0") if p]
if not paths:
    print("internal names: git ls-files listed nothing; refusing to pass", file=sys.stderr)
    sys.exit(1)

def digest(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()

def names_in(run):
    for length, digests in names.items():
        if len(run) < length:
            continue
        for start in range(len(run) - length + 1):
            if digest(run[start : start + length]) in digests:
                return True
    return False

def candidates(run):
    run = run.strip("./-")
    if not run:
        return
    yield run
    segments = [s.strip(".-") for s in run.split("/")]
    for i, segment in enumerate(segments):
        yield segment
        if i + 1 < len(segments):
            yield f"{segment}/{segments[i + 1]}"
    yield from re.split(r"[./-]+", run)

def token_in(run):
    for candidate in candidates(run):
        digests = tokens.get(len(candidate))
        if digests and digest(candidate) in digests:
            return True
    return False

LOCKFILES = re.compile(r"(^|/)(package-lock\.json|npm-shrinkwrap\.json|pnpm-lock\.yaml|[^/]*\.lock)$")

seen_names = {}
seen_tokens = {}
hits = []
for path in paths:
    if path in (names_path, tokens_path):
        continue
    try:
        data = open(path, "rb").read()
    except (IsADirectoryError, FileNotFoundError):
        continue  # a submodule or a file deleted in the work tree
    if b"\0" in data[:8192]:
        continue  # binary
    check_tokens = not LOCKFILES.search(path)
    text = data.decode("utf-8", errors="ignore").lower()
    for number, line in enumerate(text.splitlines(), 1):
        hit = False
        for run in re.findall(r"[a-z0-9]+", line):
            found = seen_names.get(run)
            if found is None:
                found = seen_names[run] = names_in(run)
            if found:
                hit = True
                break
        if not hit and check_tokens:
            for run in re.findall(r"[a-z0-9./-]+", line):
                found = seen_tokens.get(run)
                if found is None:
                    found = seen_tokens[run] = token_in(run)
                if found:
                    hit = True
                    break
        if hit:
            hits.append(f"{path}:{number}")

if hits:
    print("internal names: the public tree names an internal project again:", file=sys.stderr)
    for hit in hits:
        print(f"  {hit}", file=sys.stderr)
    print("internal names: use a neutral name, or cite the contract by its title", file=sys.stderr)
    sys.exit(1)
count = sum(len(d) for d in names.values()) + sum(len(d) for d in tokens.values())
print(f"internal names: none of the {count} listed names appear in {len(paths)} files")
PY
)
git ls-files -z | python3 -c "$scan" "$names" "$tokens"
