#!/usr/bin/env bash
# SDK black-box test harness. Boots a source-built Maidan on
# SQLite with a real member-bound token, waits for health, runs the chosen language's
# SDK test suite against it, then tears the server down.
#
# Usage:  scripts/sdk-test.sh [typescript|python|go|rust]   (default: typescript)
# Env:    MAIDAN_SDK_PORT (default 8080).
#
# Build first, then run the binary, so the health-wait covers only boot (a cold
# `cargo run` compile would otherwise outlast it).
set -euo pipefail
cd "$(dirname "$0")/.."

lang="${1:-typescript}"
port="${MAIDAN_SDK_PORT:-8080}"
base="http://127.0.0.1:${port}"

echo "=== building maidan-server ==="
cargo build --quiet --bin maidan-server --bin maidan

sdk_tmp="$(mktemp -d)"
sdk_db="sqlite://${sdk_tmp}/maidan.db?mode=rwc"
init_output="$(DATABASE_URL="${sdk_db}" ./target/debug/maidan init --workspace sdk-tests)"
admin_token="$(awk '$1 ~ /^maid_/ { print $1 }' <<<"${init_output}")"
workspace_id="$(awk '/^[[:space:]]*workspace:/ { gsub(/[()]/, "", $3); print $3 }' <<<"${init_output}")"
test -n "${admin_token}"
test -n "${workspace_id}"

echo "=== booting (SQLite, authenticated) ==="
DATABASE_URL="${sdk_db}" MAIDAN_SESSION_SECRET="sdk-test-session-secret-change-me-0123456789" \
  MAIDAN_BOOTSTRAP=1 MAIDAN_RATE_LIMIT_MAX=0 \
  MAIDAN_BIND="127.0.0.1:${port}" ./target/debug/maidan-server &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$sdk_tmp"' EXIT

up=0
for _ in $(seq 1 60); do
  if curl -sf -H "authorization: Bearer ${admin_token}" "${base}/me" >/dev/null 2>&1; then
    echo "server is up and the bootstrap token resolves"; up=1; break
  fi
  sleep 1
done
[ "$up" -eq 1 ] || { echo "server/token did not become ready within 60s" >&2; exit 1; }

export MAIDAN_URL="${base}"
export MAIDAN_TOKEN="${admin_token}"
export MAIDAN_WORKSPACE="${workspace_id}"
echo "=== running ${lang} SDK tests ==="
case "$lang" in
  typescript) (cd sdk/typescript && node --test --test-concurrency=1) ;;
  python)     (cd sdk/python && PYTHONPATH="$PWD/src:${PYTHONPATH:-}" python3 -m pytest -q) ;;
  go)         (cd sdk/go && go test ./...) ;;
  rust)       (cd sdk/rust && cargo test -- --test-threads=1) ;;
  *) echo "unknown language: ${lang}" >&2; exit 2 ;;
esac
