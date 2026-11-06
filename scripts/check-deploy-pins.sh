#!/usr/bin/env bash
# Every deploy path and install command pins the newest release: the highest
# `## [N.N.N]` CHANGELOG version whose `vN.N.N` tag exists, whatever the section order. A section added by a
# close PR before its tag is cut does not count, because a pin can only move
# once the tag's images and tarballs exist. Red after a tag is cut, until the one
# PR that bumps every pin below merges, except on the tagged commit itself (see
# below). Needs the tags (actions/checkout with fetch-depth: 0).
#
# Pins checked, each against that version:
#   helm/maidan/values-prod.yaml          image.tag
#   helm/maidan-stack/values-prod.yaml    maidan.image.tag and postgresql.image.tag
#   docker/Dockerfile.quickstart          ARG MAIDAN_VERSION (its tarball SHA-256s
#                                         move with it; they cannot be checked here)
#   compose.quickstart.yaml               the MAIDAN_VERSION default
#   k8s/overlays/prod/kustomization.yaml  newTag of maidan-server and maidan-postgres
#   listings/registry/server.json         its version and its package version
#   listings/toolhive/server.json,
#   listings/docker/server.yaml           the image they name
#   README.md, docs/Production.md         every ghcr.io/david-engelmann/maidan-{server,
#                                         cli,postgres}:vN.N.N and MAIDAN_TAG=vN.N.N
#   docs/Pi.md                            its image pins, MAIDAN_TAG=vN.N.N and the
#                                         release link the native install downloads
set -euo pipefail
cd "$(dirname "$0")/.."

die() {
  echo "deploy pins: $*" >&2
  exit 1
}

# Higher semver, compared numerically so section order cannot pick an older tag.
version_gt() {
  # Reject anything that is not dotted integers so bash arithmetic cannot
  # evaluate shell metacharacters from a corrupted CHANGELOG line.
  [[ "$1" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] || return 1
  [[ "$2" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] || return 1
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

# A release that was tagged but never published has no images to pin
# (v418.0.0's scan failed; v420.0.0's aarch64 build had no npm), so it is not
# a pin target. Each such release is one line here, with its reason.
unpublished=" $(grep -oE '^[0-9]+\.[0-9]+\.[0-9]+' scripts/unpublished-releases 2>/dev/null | tr '\n' ' ') "
best=""
while read -r version; do
  [[ "$unpublished" == *" ${version} "* ]] && continue
  if git rev-parse -q --verify "refs/tags/v${version}" >/dev/null; then
    if [[ -z "$best" ]] || version_gt "$version" "$best"; then
      best="$version"
    fi
  fi
done < <(grep -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
[[ -n "$best" ]] || die "no CHANGELOG section has its tag here (fetch the tags: fetch-depth: 0)"
expected="v${best}"
# The release record (scripts/check-release-records.sh) moves README's server
# image and MAIDAN_TAG to the release it records before that release is
# tagged, so the record commit itself can pass CI and be promoted. Those two
# README pins may name the record's version while its tag does not exist yet:
# the Capabilities head, when it is newer than every tagged section and has a
# CHANGELOG section of its own.
record="$(grep -m 1 -oE '^## \[v[0-9]+\.[0-9]+\.[0-9]+\]' docs/Capabilities.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
record_ok=""
if [[ -n "$record" ]] && version_gt "$record" "$best" \
  && ! git rev-parse -q --verify "refs/tags/v${record}" >/dev/null \
  && grep -qE "^## \[${record//./\\.}\]" CHANGELOG.md; then
  record_ok="v${record}"
fi

# The release commit is tagged before its images exist, and the pins can only
# move in a PR after it, so on that one commit (main's push CI right after a
# promotion) the pins may still name the release before it. Any later commit
# is held to the newest release.
release_alt=""
if git tag --points-at HEAD | grep -qx "v${best}"; then
  while read -r version; do
    [[ "$unpublished" == *" ${version} "* ]] && continue
    git rev-parse -q --verify "refs/tags/v${version}" >/dev/null || continue
    version_gt "$best" "$version" || continue
    if [[ -z "$release_alt" ]] || version_gt "$version" "${release_alt#v}"; then
      release_alt="v${version}"
    fi
  done < <(grep -oE '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' CHANGELOG.md | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
fi

failures=()
checked=()

# pin <file> <count> <ERE matching the pin, with vN.N.N in it>: the file has
# exactly <count> such pins (none moved out of the pattern's reach), each at
# the expected version. <count> 0 means "one or more".
pin() {
  local file="$1" count="$2" pattern="$3" alt="${4:-}" found=0 line text version
  [[ -f "$file" ]] || die "$file is missing"
  set +e
  hits=$(grep -noE "$pattern" "$file")
  grc=$?
  set -e
  if [[ "$grc" -gt 1 ]]; then
    die "$file: grep failed ($grc) for pattern ${pattern}"
  fi
  while IFS=: read -r line text; do
    [[ -n "$line" ]] || continue
    found=$((found + 1))
    version="$(grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' <<<"$text" | head -n 1)"
    checked+=("${file}:${line}")
    [[ "$version" == "$expected" || ( -n "$alt" && "$version" == "$alt" ) \
      || ( -n "$release_alt" && "$version" == "$release_alt" ) ]] ||
      failures+=("${file}:${line} pins ${version}: ${text}")
  done <<< "$hits"
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
pin README.md 0 "${image}" "$record_ok"
pin README.md 1 "^MAIDAN_TAG=${v}$" "$record_ok"
pin docs/Production.md 1 "^MAIDAN_TAG=${v}$"
# The catalog listings under listings/ name the published image too: the
# registry entry's package version, and the image in ToolHive and Docker.
pin listings/registry/server.json 1 "\"version\": \"${v}\""
pin listings/toolhive/server.json 0 "${image}"
pin listings/docker/server.yaml 0 "${image}"
# The registry entry's own version is the release without its v.
registry_version="$(grep -oE '^  "version": "[0-9]+\.[0-9]+\.[0-9]+"' listings/registry/server.json | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
[[ "v${registry_version}" == "$expected" || "v${registry_version}" == "$release_alt" ]] || \
  failures+=("listings/registry/server.json version is ${registry_version:-missing}, not ${expected#v}")
pin docs/Pi.md 0 "${image}"
pin docs/Pi.md 1 "^MAIDAN_TAG=${v}$"
pin docs/Pi.md 1 "releases/tag/${v}"

if [[ "${#failures[@]}" -gt 0 ]]; then
  echo "deploy pins: the newest tagged CHANGELOG section is ${expected}; bump every pin in one PR:" >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi
echo "deploy pins: ${#checked[@]} pins at ${expected}${release_alt:+ or ${release_alt} (this commit is the ${expected} release)}: ${checked[*]}"
