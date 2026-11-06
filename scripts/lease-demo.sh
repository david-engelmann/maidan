#!/usr/bin/env bash
# Two-language lease demo — the falsifiable hello-world. Boots a
# source-built Maidan on SQLite with real member-bound tokens, then runs
# examples/lease_demo/lease_demo.py: a Python SDK worker and a TypeScript SDK worker
# both claim tasks off one channel, and Maidan hands each task to exactly one worker.
#
# Usage:  scripts/lease-demo.sh
# Env:    MAIDAN_DEMO_PORT (an ephemeral loopback port when unset). Needs python3 + node on PATH.
#         CARGO_TARGET_DIR (the built binaries are read from there).
set -euo pipefail
cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

if [ -n "${MAIDAN_DEMO_PORT:-}" ]; then
  port="$MAIDAN_DEMO_PORT"
else
  port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
fi
base="http://127.0.0.1:${port}"

for cmd in python3 node curl; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

echo "=== building maidan-server ==="
cargo build --quiet --bin maidan-server --bin maidan
# cargo writes under CARGO_TARGET_DIR when it is set, otherwise ./target.
bin_dir="${CARGO_TARGET_DIR:-target}/debug"

demo_tmp="$(mktemp -d)"
demo_db="sqlite://${demo_tmp}/maidan.db?mode=rwc"
init_output="$(DATABASE_URL="${demo_db}" "${bin_dir}/maidan" init --workspace lease-demo)"
admin_token="$(awk '$1 ~ /^maid_/ { print $1 }' <<<"${init_output}")"
workspace_id="$(awk '/^[[:space:]]*workspace:/ { gsub(/[()]/, "", $3); print $3 }' <<<"${init_output}")"
test -n "${admin_token}"
test -n "${workspace_id}"

echo "=== booting (SQLite, authenticated) ==="
DATABASE_URL="${demo_db}" MAIDAN_SESSION_SECRET="lease-demo-session-secret-change-me-0123456789" \
  MAIDAN_BOOTSTRAP=1 MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_BIND="127.0.0.1:${port}" "${bin_dir}/maidan-server" >"${demo_tmp}/server.log" 2>&1 &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$demo_tmp"' EXIT

up=0
for _ in $(seq 1 300); do
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "maidan-server exited early" >&2
    cat "${demo_tmp}/server.log" >&2 || true
    exit 1
  fi
  if curl -sf -H "authorization: Bearer ${admin_token}" "${base}/me" >/dev/null 2>&1; then
    echo "server is up and the bootstrap token resolves"; up=1; break
  fi
  sleep 0.1
done
if [ "$up" != "1" ]; then
  echo "server/token did not become ready within 30s" >&2
  cat "${demo_tmp}/server.log" >&2 || true
  exit 1
fi

echo "=== running the two-language lease demo ==="
MAIDAN_URL="${base}" MAIDAN_TOKEN="${admin_token}" MAIDAN_WORKSPACE="${workspace_id}" \
  PYTHONPATH="sdk/python/src:${PYTHONPATH:-}" \
  python3 examples/lease_demo/lease_demo.py
