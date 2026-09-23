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

capability_version="$(first_match '^## v([0-9]+\.[0-9]+\.[0-9]+)( .*)?$' docs/Capabilities.md)"
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

if [[ "${1:-}" == "--tag" ]]; then
  [[ $# -eq 2 ]] || die "usage: $0 [--tag vX.Y.Z]"
  [[ "$2" == "v$expected" ]] || \
    die "release tag $2 does not match the newest source record v$expected"
elif [[ $# -ne 0 ]]; then
  die "usage: $0 [--tag vX.Y.Z]"
fi

echo "release-record contract: v$expected"
