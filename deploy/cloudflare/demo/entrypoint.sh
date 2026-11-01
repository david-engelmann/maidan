#!/usr/bin/env bash
# Boot the try.maidan.world demo: a fresh SQLite workspace seeded by
# scripts/demo-board.sh, then the public listener.
#
# 1. Fresh keys for this boot. Nothing outlives the container, so nothing needs
#    a stored secret: the reset (the Worker destroying the container) throws
#    the keys away with the data.
# 2. The seed runs against a loopback listener with MAIDAN_BOOTSTRAP=1, which
#    the script needs to create members. It runs outside production mode,
#    which would refuse the plain-http loopback origin.
# 3. That listener stops, and the public one starts without bootstrap.
set -euo pipefail

rm -f /tmp/maidan-demo.db*
rm -rf "${ARTIFACT_LOCALFS_ROOT:-/tmp/maidan-demo-artifacts}"
mkdir -p "${ARTIFACT_LOCALFS_ROOT:-/tmp/maidan-demo-artifacts}"

MAIDAN_CONTENT_KEK=$(openssl rand -hex 32)
MAIDAN_SESSION_SECRET=$(openssl rand -hex 32)
export MAIDAN_CONTENT_KEK MAIDAN_SESSION_SECRET

seed_port=8081
env -u MAIDAN_ENV MAIDAN_BOOTSTRAP=1 MAIDAN_BIND="127.0.0.1:$seed_port" MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_PUBLIC_ORIGIN="http://127.0.0.1:$seed_port" maidan-server &
seed_pid=$!
for _ in $(seq 1 300); do
  curl -fsS "http://127.0.0.1:$seed_port/health" >/dev/null 2>&1 && break
  sleep 0.2
done

MAIDAN_URL="http://127.0.0.1:$seed_port" MAIDAN_WORKSPACE_NAME=demo \
  demo-board.sh >&2

kill "$seed_pid"
wait "$seed_pid" || true

exec maidan-server
