#!/usr/bin/env bash
# Two-language lease demo — the falsifiable hello-world. Boots a
# source-built Maidan on SQLite with real member-bound tokens, then runs
# examples/lease_demo/lease_demo.py: a Python SDK worker and a TypeScript SDK worker
# both claim tasks off one channel, and Maidan hands each task to exactly one worker.
#
# Usage:  scripts/lease-demo.sh
# Env:    MAIDAN_DEMO_PORT (default 8080). Needs python3 + node on PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

port="${MAIDAN_DEMO_PORT:-8080}"
base="http://127.0.0.1:${port}"

for cmd in python3 node curl; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

echo "=== building maidan-server ==="
cargo build --quiet --bin maidan-server --bin maidan

demo_tmp="$(mktemp -d)"
demo_db="sqlite://${demo_tmp}/maidan.db?mode=rwc"
init_output="$(DATABASE_URL="${demo_db}" ./target/debug/maidan init --workspace lease-demo)"
admin_token="$(awk '$1 ~ /^maid_/ { print $1 }' <<<"${init_output}")"
workspace_id="$(awk '/^[[:space:]]*workspace:/ { gsub(/[()]/, "", $3); print $3 }' <<<"${init_output}")"
test -n "${admin_token}"
test -n "${workspace_id}"

echo "=== booting (SQLite, authenticated) ==="
DATABASE_URL="${demo_db}" MAIDAN_SESSION_SECRET="lease-demo-session-secret-change-me-0123456789" \
  MAIDAN_BOOTSTRAP=1 MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_BIND="127.0.0.1:${port}" ./target/debug/maidan-server &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$demo_tmp"' EXIT

up=0
for _ in $(seq 1 60); do
  if curl -sf -H "authorization: Bearer ${admin_token}" "${base}/me" >/dev/null 2>&1; then
    echo "server is up and the bootstrap token resolves"; up=1; break
  fi
  sleep 1
done
[ "$up" -eq 1 ] || { echo "server/token did not become ready within 60s" >&2; exit 1; }

echo "=== running the two-language lease demo ==="
MAIDAN_URL="${base}" MAIDAN_TOKEN="${admin_token}" MAIDAN_WORKSPACE="${workspace_id}" \
  PYTHONPATH="sdk/python/src:${PYTHONPATH:-}" \
  python3 examples/lease_demo/lease_demo.py
