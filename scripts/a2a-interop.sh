#!/usr/bin/env bash
# A2A v1.0 interop / conformance harness (Cluster 289).
#
# Boots a source-built Maidan on SQLite (auth disabled, dev-only), waits for
# health, runs the A2A conformance client (examples/a2a_interop.py) against it,
# then tears the server down. Exits non-zero if any conformance check fails.
#
# Like scripts/loadgen.sh and scripts/chaos.sh this is a reproducible harness for
# local / manual use (and a report-only CI job); it is not a required gate.
#
# Usage:
#   pip install "httpx>=0.27"
#   scripts/a2a-interop.sh
#
# Env: MAIDAN_A2A_PORT (default 8080), MAIDAN_A2A_GRPC_PORT (default 50251).
set -euo pipefail

cd "$(dirname "$0")/.."

port="${MAIDAN_A2A_PORT:-8080}"
grpc_port="${MAIDAN_A2A_GRPC_PORT:-50251}"
base="http://127.0.0.1:${port}"

# Build first (blocking) so the health-wait below only covers boot, not compile —
# a cold `cargo run` compile in CI would otherwise outlast the wait and the client
# would hit a server that isn't up yet.
echo "=== building maidan-server ==="
cargo build --quiet --bin maidan-server

echo "=== booting (SQLite, auth disabled) ==="
DATABASE_URL="sqlite::memory:" \
  AUTH_DISABLED=1 MAIDAN_ALLOW_INSECURE_NO_AUTH=1 \
  MAIDAN_BIND="127.0.0.1:${port}" \
  MAIDAN_A2A_GRPC_ADDR="127.0.0.1:${grpc_port}" \
  MAIDAN_A2A_PUBLIC_ORIGIN="${base}" \
  MAIDAN_A2A_GRPC_PUBLIC_ADDR="127.0.0.1:${grpc_port}" \
  ./target/debug/maidan-server &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true' EXIT

echo "=== waiting for /health ==="
up=0
for _ in $(seq 1 60); do
  if curl -sf "${base}/health" >/dev/null 2>&1; then
    echo "server is up"
    up=1
    break
  fi
  sleep 1
done
if [ "$up" -ne 1 ]; then
  echo "server did not become healthy within 60s" >&2
  exit 1
fi

echo "=== running A2A conformance client ==="
MAIDAN_URL="${base}" python3 examples/a2a_interop.py
