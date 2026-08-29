#!/usr/bin/env bash
# Two-language lease demo (Cluster 317) — the falsifiable hello-world. Boots a
# source-built Maidan on SQLite (auth disabled, dev-only, like sdk-test.sh), then runs
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
cargo build --quiet --bin maidan-server

echo "=== booting (SQLite, auth disabled — dev only) ==="
DATABASE_URL="sqlite::memory:" AUTH_DISABLED=1 MAIDAN_ALLOW_INSECURE_NO_AUTH=1 \
  MAIDAN_BIND="127.0.0.1:${port}" ./target/debug/maidan-server &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true' EXIT

up=0
for _ in $(seq 1 60); do
  if curl -sf "${base}/health" >/dev/null 2>&1; then echo "server is up"; up=1; break; fi
  sleep 1
done
[ "$up" -eq 1 ] || { echo "server did not become healthy within 60s" >&2; exit 1; }

echo "=== running the two-language lease demo ==="
MAIDAN_URL="${base}" PYTHONPATH="sdk/python/src:${PYTHONPATH:-}" \
  python3 examples/lease_demo/lease_demo.py
