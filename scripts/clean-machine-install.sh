#!/usr/bin/env bash
# Clean-machine install check (Next 14): from nothing to a running Maidan, using
# only what a release publishes. No clone, no Rust toolchain, no source build.
#
#   curl -fsSLO https://raw.githubusercontent.com/david-engelmann/maidan/dev/scripts/clean-machine-install.sh
#   bash clean-machine-install.sh            # latest release
#   MAIDAN_TAG=v419.0.0 bash clean-machine-install.sh
#   MAIDAN_REF=dev bash clean-machine-install.sh   # quickstart files from a branch
#
# What it does, in a fresh temp directory:
#   1. Resolves the release tag (MAIDAN_TAG, or the latest GitHub release).
#   2. Downloads compose.quickstart.yaml, docker/Dockerfile.quickstart and
#      scripts/quickstart-two-agents.sh at MAIDAN_REF (default: the tag). The
#      Dockerfile pulls the release binary those files pin and checks its SHA-256,
#      so the stack runs a published release. If the pin is not the tag, it says
#      so, and fails under STRICT=1.
#   3. Starts it under its own compose project name on a free loopback port.
#   4. Seeds the first admin with the bundled `maidan init`, then runs the
#      two-agent demo against it.
#   5. Checks /health reports the release version, and prints the published GHCR
#      server image digest for the validation record.
#   6. Tears the stack down (containers, volume, built image) unless KEEP=1.
#
# Needs: docker (with compose v2), curl, jq. The admin token stays in a 0600 file
# in the temp directory and is never printed.
set -euo pipefail

REPO="${MAIDAN_REPO:-david-engelmann/maidan}"
PROJECT="${MAIDAN_COMPOSE_PROJECT:-maidan-clean-install-$$}"
KEEP="${KEEP:-0}"

for cmd in docker curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done
docker compose version >/dev/null 2>&1 || { echo "docker compose v2 is required" >&2; exit 1; }

TAG="${MAIDAN_TAG:-}"
if [ -z "$TAG" ]; then
  TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | jq -er '.tag_name')
fi
echo "release: ${TAG}"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/maidan-clean-install.XXXXXX")
compose() { docker compose -p "$PROJECT" -f "$WORK/compose.quickstart.yaml" "$@"; }
cleanup() {
  if [ "$KEEP" = "1" ]; then
    echo "KEEP=1: stack left running as compose project ${PROJECT} in ${WORK}"
    return
  fi
  compose down -v --rmi local >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

REF="${MAIDAN_REF:-$TAG}"
raw="https://raw.githubusercontent.com/${REPO}/${REF}"
mkdir -p "$WORK/docker" "$WORK/scripts"
for f in compose.quickstart.yaml docker/Dockerfile.quickstart scripts/quickstart-two-agents.sh; do
  curl -fsSL "${raw}/${f}" -o "$WORK/${f}"
done
chmod +x "$WORK/scripts/quickstart-two-agents.sh"

# A free loopback port, so a taken 8080 never fails the install.
port="${MAIDAN_HOST_PORT:-}"
if [ -z "$port" ]; then
  for p in $(seq 18080 18180); do
    if ! curl -s -o /dev/null --max-time 1 "http://127.0.0.1:${p}/"; then port=$p; break; fi
  done
fi
[ -n "$port" ] || { echo "no free loopback port in 18080-18180" >&2; exit 1; }

# The pinned binary and its SHA-256 live together in the downloaded files, so the
# version is read from them, never overridden: a mismatched pair fails the build.
pinned=$(sed -n 's/.*MAIDAN_VERSION:-\(v[0-9][0-9.]*\)}.*/\1/p' "$WORK/compose.quickstart.yaml" | head -n 1)
[ -n "$pinned" ] || { echo "could not read the pinned MAIDAN_VERSION from compose.quickstart.yaml" >&2; exit 1; }
echo "quickstart files at ${REF} pin ${pinned}"
if [ "$pinned" != "$TAG" ]; then
  echo "WARN: the quickstart at ${REF} pins ${pinned}, not ${TAG}" >&2
  [ "${STRICT:-0}" = "1" ] && exit 1
fi
unset MAIDAN_VERSION
export MAIDAN_HOST_PORT="$port"
echo "starting compose project ${PROJECT} on 127.0.0.1:${port}"
compose up -d --build --wait

init_out="$WORK/init.out"
( umask 077; compose exec -T maidan maidan init --workspace clean-install >"$init_out" )
workspace=$(sed -n 's/^  workspace: .*(\([0-9a-f-]\{36\}\))$/\1/p' "$init_out")
token=$(awk '/Admin bearer token/{f=1;next} f && /^    [^ ]+$/{print $1; exit}' "$init_out")
[ -n "$workspace" ] && [ -n "$token" ] || { echo "could not read maidan init output" >&2; exit 1; }
echo "seeded workspace ${workspace}"

base="http://127.0.0.1:${port}"
MAIDAN_URL="$base" MAIDAN_TOKEN="$token" MAIDAN_WORKSPACE="$workspace" \
  "$WORK/scripts/quickstart-two-agents.sh"

want="v${pinned#v}"
health=$(curl -fsS "${base}/health")
got=$(printf '%s' "$health" | jq -r '.version // empty')
got="v${got#v}"
if [ "$got" != "$want" ]; then
  echo "FAIL: /health reports version '${got}', expected '${want}'" >&2
  exit 1
fi

digest=$(docker buildx imagetools inspect "ghcr.io/${REPO}-server:${pinned}" --format '{{json .Manifest}}' 2>/dev/null \
  | jq -r '.digest // empty' || true)
echo
echo "PASS: clean-machine install of ${pinned} from ${REF} (server reports ${got})"
echo "  published server image: ghcr.io/${REPO}-server:${pinned}${digest:+@${digest}}"
