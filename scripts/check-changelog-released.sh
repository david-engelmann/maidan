#!/usr/bin/env bash
# A released CHANGELOG section only changes by being corrected, never by
# gaining entries: new work goes under [Unreleased]. For the newest `## [N.0.0]`
# section whose `vN.0.0` tag exists, the `### ` headings must be the ones the
# section had at that tag. Two entries were filed under [412.0.0] after its tag
# on 2026-09-30, by placing them before a heading that happened to sit below
# the release line. Needs the tags (actions/checkout with fetch-depth: 0).
set -euo pipefail
cd "$(dirname "$0")/.."

section_headings() { # $1 = file, $2 = version: the ### headings of ## [version]
  awk -v v="## [$2]" 'index($0, v) == 1 {on=1; next} on && /^## \[/ {exit} on && /^### / {print}' "$1"
}

# An entry appears once. A keep-both merge of a moved entry once left two
# copies of the same entry in [Unreleased].
dups=$(awk '/^## \[Unreleased\]/{on=1; next} on && /^## \[/{exit} on && /^### /' CHANGELOG.md \
  | grep -vxE '### (Added|Changed|Fixed|Removed|Security|Deprecated)' | sort | uniq -d || true)
if [[ -n "$dups" ]]; then
  echo "changelog: [Unreleased] lists an entry more than once:"
  printf '%s\n' "$dups" | sed 's/^/  /'
  exit 1
fi

version=$(grep -m1 -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)
[[ -n "$version" ]] || { echo "changelog: no released section"; exit 0; }
if ! git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
  echo "changelog: v$version is not tagged; nothing to compare"; exit 0
fi
tagged=$(mktemp); trap 'rm -f "$tagged"' EXIT
git show "v$version:CHANGELOG.md" > "$tagged"
added=$(comm -13 <(section_headings "$tagged" "$version" | sort) <(section_headings CHANGELOG.md "$version" | sort))
if [[ -n "$added" ]]; then
  echo "changelog: [$version] gained entries after v$version was tagged; move them to [Unreleased]:"
  printf '%s\n' "$added" | sed 's/^/  /'
  exit 1
fi
echo "changelog: [$version] matches v$version"
