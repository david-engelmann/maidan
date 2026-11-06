#!/usr/bin/env bash
# Boot the try.maidan.world demo: an empty SQLite database and the public
# listener, which hands each visitor a sandbox of their own (POST /sandbox,
# MAIDAN_DEMO_SANDBOX=1 in deploy/cloudflare/src/demo-config.js).
#
# Fresh keys for this boot. Nothing outlives the container, so nothing needs a
# stored secret: the reset (the Worker destroying the container) throws the
# keys away with the data. Nothing is seeded, and no listener ever runs with
# bootstrap on: the image is the stock server, which has no bootstrap routes.
set -euo pipefail

root="${ARTIFACT_LOCALFS_ROOT:-/tmp/maidan-demo-artifacts}"
# The demo only writes under /tmp. Refuse a path that escapes it, walks up, or
# carries shell-metacharacters a later operator might expand.
case "$root" in
  /tmp/*) ;;
  *)
    echo "ARTIFACT_LOCALFS_ROOT must be an absolute path under /tmp" >&2
    exit 1
    ;;
esac
case "$root" in
  *..*|*$'\n'*|*$'\r'*|*'*'*|*'?'*|*'{'*)
    echo "ARTIFACT_LOCALFS_ROOT is not a safe path: $root" >&2
    exit 1
    ;;
esac

rm -f /tmp/maidan-demo.db*
rm -rf "$root"
mkdir -p "$root"
export ARTIFACT_LOCALFS_ROOT="$root"

MAIDAN_CONTENT_KEK=$(openssl rand -hex 32)
MAIDAN_SESSION_SECRET=$(openssl rand -hex 32)
# openssl must actually produce 64 hex chars; an empty export would boot the
# server with blank secrets.
[[ "$MAIDAN_CONTENT_KEK" =~ ^[0-9a-f]{64}$ ]] || {
  echo "failed to mint MAIDAN_CONTENT_KEK" >&2
  exit 1
}
[[ "$MAIDAN_SESSION_SECRET" =~ ^[0-9a-f]{64}$ ]] || {
  echo "failed to mint MAIDAN_SESSION_SECRET" >&2
  exit 1
}
export MAIDAN_CONTENT_KEK MAIDAN_SESSION_SECRET

exec maidan-server
