#!/usr/bin/env bash
# Changelog fragments (changelog.d/*.md, one per change) and the release fold.
#   check            every fragment is one `### ` section of Keep a Changelog bullets
#   preview          print the fragments as [Unreleased] would show them
#   release X.Y.Z    fold [Unreleased] and every fragment into `## [X.Y.Z] — today`
set -euo pipefail
cd "$(dirname "$0")/.."

fragments() { # oldest first by the commit that added each, then by name
  local f
  for f in changelog.d/*.md; do
    [[ -e $f && $(basename "$f") != README.md ]] || continue
    printf '%s\t%s\n' "$(git log --diff-filter=A --format=%ct -1 -- "$f" 2>/dev/null || echo 0)" "$f"
  done | sort -k1,1n -k2,2 | cut -f2
}

check() {
  local f bad=0
  while read -r f; do
    [[ -n $f ]] || continue
    local first heads
    first=$(grep -m1 -v '^[[:space:]]*$' "$f" || true)
    heads=$(grep -c '^### ' "$f" || true)
    if [[ $first != '### '* ]]; then
      echo "changelog.d: $f must start with a '### ' heading"; bad=1
    fi
    if [[ $heads -ne 1 ]]; then
      echo "changelog.d: $f must hold exactly one '### ' section (found $heads)"; bad=1
    fi
    if grep -q '^## ' "$f"; then
      echo "changelog.d: $f must not contain a '## ' heading"; bad=1
    fi
    if ! grep -qE '^- \*\*(Added|Changed|Fixed|Removed|Security|Deprecated):\*\*' "$f"; then
      echo "changelog.d: $f needs at least one '- **Added:**' (or Changed, Fixed, ...) bullet"; bad=1
    fi
  done < <(fragments)
  # The same heading twice, across fragments and [Unreleased], is one change
  # filed twice.
  local dups
  dups=$( { awk '/^## \[Unreleased\]/{on=1; next} on && /^## \[/{exit} on && /^### /' CHANGELOG.md
            while read -r f; do [[ -n $f ]] && grep '^### ' "$f"; done < <(fragments); } | sort | uniq -d )
  if [[ -n $dups ]]; then
    echo "changelog.d: a heading is filed more than once:"; printf '  %s\n' "$dups"; bad=1
  fi
  [[ $bad -eq 0 ]] && echo "changelog.d: $(fragments | grep -c . || true) fragment(s) ok"
  return $bad
}

preview() {
  local f
  while read -r f; do
    [[ -n $f ]] || continue
    sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' "$f"; echo
  done < <(fragments)
}

release() {
  local version=$1 today
  [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "usage: $0 release X.Y.Z"; exit 2; }
  grep -q "^## \[$version\]" CHANGELOG.md && { echo "changelog: [$version] already exists"; exit 1; }
  check >/dev/null
  today=$(date -u +%Y-%m-%d)
  local body; body=$(mktemp); trap 'rm -f "$body"' RETURN
  preview > "$body"
  python3 - "$version" "$today" "$body" <<'PY'
import sys
version, today, body_path = sys.argv[1:4]
body = open(body_path).read().strip()
s = open("CHANGELOG.md").read()
head = "## [Unreleased]\n"
i = s.index(head) + len(head)
j = s.index("\n## [", i)
unreleased = s[i:j].strip()
parts = [p for p in (unreleased, body) if p]
section = f"## [{version}] — {today}\n\n" + "\n\n".join(parts) + "\n"
s = s[:i] + "\n" + section + s[j:]
open("CHANGELOG.md", "w").write(s)
PY
  local f
  while read -r f; do
    [[ -n $f ]] || continue
    git rm -q -- "$f" 2>/dev/null || rm -f -- "$f"
  done < <(fragments)
  echo "changelog: folded [Unreleased] and the fragments into [$version]; write its summary paragraph"
}

case "${1:-}" in
  check) check ;;
  preview) preview ;;
  release) release "${2:-}" ;;
  *) echo "usage: $0 check | preview | release X.Y.Z"; exit 2 ;;
esac
