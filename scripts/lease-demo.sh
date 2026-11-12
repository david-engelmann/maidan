#!/usr/bin/env bash
# Two-language lease demo — the falsifiable hello-world. Boots a
# source-built Maidan on SQLite with real member-bound tokens, then runs
# examples/lease_demo/lease_demo.py: a Python SDK worker and a TypeScript SDK worker
# both claim tasks off one channel, and Maidan hands each task to exactly one worker.
#
# Usage:  scripts/lease-demo.sh
# Env:    MAIDAN_DEMO_PORT (default 8080). Needs python3 + node on PATH.
#         CARGO_TARGET_DIR (the built binaries are read from there).
set -euo pipefail
cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

port="${MAIDAN_DEMO_PORT:-8080}"
base="http://127.0.0.1:${port}"

for cmd in python3 node curl; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

echo "=== building maidan-server ==="
cargo build --quiet --bin maidan-server
# cargo writes under CARGO_TARGET_DIR when it is set, otherwise ./target.
bin_dir="${CARGO_TARGET_DIR:-target}/debug"

demo_tmp="$(mktemp -d)"
demo_db="sqlite://${demo_tmp}/maidan.db?mode=rwc"
# A per-run operator token: boot opens the empty database's first workspace
# with it, and it opens the demo's workspace below.
dev_admin_token="$(od -An -tx1 -N24 /dev/urandom | tr -d ' \n')"

echo "=== booting (SQLite, authenticated) ==="
DATABASE_URL="${demo_db}" MAIDAN_SESSION_SECRET="lease-demo-session-secret-change-me-0123456789" \
  MAIDAN_DEV_ADMIN_TOKEN="${dev_admin_token}" MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_BIND="127.0.0.1:${port}" "${bin_dir}/maidan-server" &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$demo_tmp"' EXIT

up=0
for _ in $(seq 1 60); do
  if curl -sf -H "authorization: Bearer ${dev_admin_token}" "${base}/me" >/dev/null 2>&1; then
    echo "server is up and the dev admin token resolves"; up=1; break
  fi
  sleep 1
done
[ "$up" -eq 1 ] || { echo "server/token did not become ready within 60s" >&2; exit 1; }

# The demo's workspace and its admin, who connects the two agents.
{ read -r admin_token; read -r workspace_id; } < <(
  curl -sf -X POST -H "authorization: Bearer ${dev_admin_token}" \
    -H "content-type: application/json" \
    -d '{"name":"lease-demo","admin_handle":"admin"}' "${base}/operator/workspaces" \
    | python3 -c 'import json,sys; b=json.load(sys.stdin); print(b["token"]["secret"]); print(b["workspace"]["id"])'
)
test -n "${admin_token}"
test -n "${workspace_id}"

echo "=== running the two-language lease demo ==="
MAIDAN_URL="${base}" MAIDAN_TOKEN="${admin_token}" MAIDAN_WORKSPACE="${workspace_id}" \
  PYTHONPATH="sdk/python/src:${PYTHONPATH:-}" \
  python3 examples/lease_demo/lease_demo.py
