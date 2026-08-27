#!/usr/bin/env bash
# SDK black-box test harness (Cluster 294+). Boots a source-built Maidan on
# SQLite (auth disabled, dev-only), waits for health, runs the chosen language's
# SDK test suite against it, then tears the server down.
#
# Usage:  scripts/sdk-test.sh [typescript|python|go|rust]   (default: typescript)
# Env:    MAIDAN_SDK_PORT (default 8080).
#
# Build first, then run the binary, so the health-wait covers only boot (a cold
# `cargo run` compile would otherwise outlast it — see Cluster 290).
set -euo pipefail
cd "$(dirname "$0")/.."

lang="${1:-typescript}"
port="${MAIDAN_SDK_PORT:-8080}"
base="http://127.0.0.1:${port}"

echo "=== building maidan-server ==="
cargo build --quiet --bin maidan-server

echo "=== booting (SQLite, auth disabled) ==="
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

export MAIDAN_URL="${base}"
echo "=== running ${lang} SDK tests ==="
case "$lang" in
  typescript) (cd sdk/typescript && node --test) ;;
  python)     (cd sdk/python && python3 -m pytest -q) ;;
  go)         (cd sdk/go && go test ./...) ;;
  rust)       (cd sdk/rust && cargo test) ;;
  *) echo "unknown language: ${lang}" >&2; exit 2 ;;
esac
