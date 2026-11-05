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

rm -f /tmp/maidan-demo.db*
rm -rf "${ARTIFACT_LOCALFS_ROOT:-/tmp/maidan-demo-artifacts}"
mkdir -p "${ARTIFACT_LOCALFS_ROOT:-/tmp/maidan-demo-artifacts}"

MAIDAN_CONTENT_KEK=$(openssl rand -hex 32)
MAIDAN_SESSION_SECRET=$(openssl rand -hex 32)
export MAIDAN_CONTENT_KEK MAIDAN_SESSION_SECRET

exec maidan-server
