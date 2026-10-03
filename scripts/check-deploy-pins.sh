#!/usr/bin/env bash
# Every deploy path and install command pins the newest release: the highest
# `## [N.N.N]` CHANGELOG version whose `vN.N.N` tag exists, whatever the section order. A section added by a
# close PR before its tag is cut does not count, because a pin can only move
# once the tag's images and tarballs exist. Red after a tag is cut, until the one
# PR that bumps every pin below merges. Needs the tags (actions/checkout with
# fetch-depth: 0).
#
# Pins checked, each against that version:
#   helm/maidan/values-prod.yaml          image.tag
#   helm/maidan-stack/values-prod.yaml    maidan.image.tag and postgresql.image.tag
#   docker/Dockerfile.quickstart          ARG MAIDAN_VERSION (its tarball SHA-256s
#                                         move with it; they cannot be checked here)
#   compose.quickstart.yaml               the MAIDAN_VERSION default
#   k8s/overlays/prod/kustomization.yaml  newTag of maidan-server and maidan-postgres
#   README.md, docs/Production.md         every ghcr.io/david-engelmann/maidan-{server,
#                                         cli,postgres}:vN.N.N and MAIDAN_TAG=vN.N.N,
#                                         and README's "pins (`vN.N.N`)" quickstart line
# docs/Pi.md is not checked yet: it still runs v315.0.0 and is rewritten with the
# next pin bump (Open Work B4), which adds it here.
set -euo pipefail
cd "$(dirname "$0")/.."

die() {
  echo "deploy pins: $*" >&2
  exit 1
}

# Higher semver, compared numerically so section order cannot pick an older tag.
version_gt() {
  local IFS=.
  local -a a=($1) b=($2)
  local i
  for i in 0 1 2; do
    if (( 10#${a[i]:-0} > 10#${b[i]:-0} )); then
      return 0
    fi
    if (( 10#${a[i]:-0} < 10#${b[i]:-0} )); then
      return 1
    fi
  done
  return 1
}

best=""
while read -r version; do
  if git rev-parse -q --verify "refs/tags/v${version}" >/dev/null; then
    if [[ -z "$best" ]] || version_gt "$version" "$best"; then
      best="$version"
    fi
  fi
done < <(grep -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
[[ -n "$best" ]] || die "no CHANGELOG section has its tag here (fetch the tags: fetch-depth: 0)"
expected="v${best}"

failures=()
checked=()

# pin <file> <count> <ERE matching the pin, with vN.N.N in it>: the file has
# exactly <count> such pins (none moved out of the pattern's reach), each at
# the expected version. <count> 0 means "one or more".
pin() {
  local file="$1" count="$2" pattern="$3" found=0 line text version
  [[ -f "$file" ]] || die "$file is missing"
  while IFS=: read -r line text; do
    [[ -n "$line" ]] || continue
    found=$((found + 1))
    version="$(grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' <<<"$text" | head -n 1)"
    checked+=("${file}:${line}")
    [[ "$version" == "$expected" ]] || failures+=("${file}:${line} pins ${version}: ${text}")
  done < <(grep -noE "$pattern" "$file" || true)
  if [[ "$count" -eq 0 ]]; then
    [[ "$found" -gt 0 ]] || die "$file: no pin matches ${pattern}; if it moved, update this script"
  else
    [[ "$found" -eq "$count" ]] || \
      die "$file: expected ${count} pin(s) matching ${pattern}, found ${found}; if one moved, update this script"
  fi
}

v='v[0-9]+\.[0-9]+\.[0-9]+'
image="ghcr\.io/david-engelmann/maidan-(server|cli|postgres):${v}"

pin helm/maidan/values-prod.yaml 1 "^  tag: ${v}$"
# maidan.image.tag (the server) and postgresql.image.tag (maidan-postgres).
pin helm/maidan-stack/values-prod.yaml 2 "^    tag: ${v}$"
pin docker/Dockerfile.quickstart 1 "^ARG MAIDAN_VERSION=${v}$"
pin compose.quickstart.yaml 1 "MAIDAN_VERSION: \\$\\{MAIDAN_VERSION:-${v}\\}"
pin k8s/overlays/prod/kustomization.yaml 2 "^    newTag: ${v}$"
pin README.md 0 "${image}"
pin README.md 1 "^MAIDAN_TAG=${v}$"
pin README.md 1 "pins \(\`${v}\`\)"
pin docs/Production.md 1 "^MAIDAN_TAG=${v}$"

if [[ "${#failures[@]}" -gt 0 ]]; then
  echo "deploy pins: the newest tagged CHANGELOG section is ${expected}; bump every pin in one PR:" >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi
echo "deploy pins: ${#checked[@]} pins at ${expected}: ${checked[*]}"
