#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

die() {
  echo "release-record contract: $*" >&2
  exit 1
}

first_match() {
  local pattern="$1"
  local file="$2"
  grep -m 1 -E "$pattern" "$file" | sed -E "s|${pattern}|\\1|"
}

requested_tag=""
if [[ "${1:-}" == "--tag" ]]; then
  [[ $# -eq 2 ]] || die "usage: $0 [--tag vX.Y.Z]"
  requested_tag="$2"
elif [[ $# -ne 0 ]]; then
  die "usage: $0 [--tag vX.Y.Z]"
fi

capability_version="$(first_match '^## \[v([0-9]+\.[0-9]+\.[0-9]+)\]\(https://github\.com/david-engelmann/maidan/releases/tag/v[0-9]+\.[0-9]+\.[0-9]+\)( .*)?$' docs/Capabilities.md)"
changelog_version="$(first_match '^## \[([0-9]+\.[0-9]+\.[0-9]+)\]( .*)?$' CHANGELOG.md)"
# Backticks and asterisks are regex literals here, not shell interpolation.
# shellcheck disable=SC2016
guide_version="$(first_match '.*latest \*\*`v([0-9]+\.[0-9]+\.[0-9]+)`\*\*.*' CLAUDE.md)"
server_image_version="$(first_match '.*ghcr\.io/david-engelmann/maidan-server:v([0-9]+\.[0-9]+\.[0-9]+).*' README.md)"
cli_image_version="$(first_match '^MAIDAN_TAG=v([0-9]+\.[0-9]+\.[0-9]+)$' README.md)"

for value_name in capability_version changelog_version guide_version server_image_version cli_image_version; do
  value="${!value_name}"
  [[ -n "$value" ]] || die "could not read ${value_name}"
done

expected="$capability_version"
for record in \
  "CHANGELOG.md:$changelog_version" \
  "CLAUDE.md:$guide_version" \
  "README server image:$server_image_version" \
  "README CLI image:$cli_image_version"; do
  name="${record%%:*}"
  version="${record#*:}"
  [[ "$version" == "$expected" ]] || \
    die "$name names v$version, but docs/Capabilities.md starts at v$expected"
done

if rg -n 'ghcr\.io/david-engelmann/maidan-(server|cli):latest' README.md >/dev/null; then
  die "README executable examples must use an immutable version tag, not :latest"
fi

heading_count=0
while IFS= read -r heading; do
  if [[ "$heading" =~ ^##\ \[v([0-9]+\.[0-9]+\.[0-9]+)\]\(https://github\.com/david-engelmann/maidan/releases/tag/v([0-9]+\.[0-9]+\.[0-9]+)\) ]]; then
    record_version="${BASH_REMATCH[1]}"
    link_version="${BASH_REMATCH[2]}"
    [[ "$record_version" == "$link_version" ]] || \
      die "capability v$record_version links to release v$link_version"
    if ! git rev-parse -q --verify "refs/tags/v$record_version" >/dev/null; then
      # A close PR prepares the newest complete record before its tag can exist.
      # Tag-triggered validation never gets this exception.
      [[ -z "$requested_tag" && "$record_version" == "$expected" ]] || \
        die "capability v$record_version is presented as released, but its tag is absent"
    fi
  elif [[ "$heading" =~ ^##\ Cluster\ ([0-9]+)\ \(source\ record\;\ no\ \`v([0-9]+\.[0-9]+\.[0-9]+)\`\ tag\) ]]; then
    cluster="${BASH_REMATCH[1]}"
    record_version="${BASH_REMATCH[2]}"
    [[ "$record_version" == "$cluster.0.0" ]] || \
      die "source-only Cluster $cluster names inconsistent v$record_version"
    if git rev-parse -q --verify "refs/tags/v$record_version" >/dev/null; then
      die "source-only Cluster $cluster has a real v$record_version tag"
    fi
  else
    die "non-canonical capability heading: $heading"
  fi
  heading_count=$((heading_count + 1))
done < <(rg '^## (\[?v[0-9]|Cluster [0-9])' docs/Capabilities.md)
[[ "$heading_count" -gt 0 ]] || die "no capability records found"

while IFS= read -r tag; do
  rg -F "releases/tag/$tag)" docs/Capabilities.md >/dev/null || \
    die "published tag $tag is not searchable in docs/Capabilities.md"
done < <(git tag --list 'v*.*.*')

while IFS= read -r linked_tag; do
  if ! git rev-parse -q --verify "refs/tags/$linked_tag" >/dev/null; then
    [[ -z "$requested_tag" && "$linked_tag" == "v$expected" ]] || \
      die "release stream links absent tag $linked_tag"
  fi
done < <(
  rg -o 'https://github\.com/david-engelmann/maidan/releases/tag/v[0-9]+\.[0-9]+\.[0-9]+' docs/Capabilities.md |
    sed 's|.*/||' |
    sort -u
)

if [[ -n "$requested_tag" ]]; then
  [[ "$requested_tag" == "v$expected" ]] || \
    die "release tag $requested_tag does not match the newest source record v$expected"
fi

echo "release-record contract: v$expected"
