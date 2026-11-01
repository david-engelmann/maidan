#!/usr/bin/env bash
# The official third-party MCP SDKs as clients of a seeded Maidan.
#
# mcp-inspector.sh drives the Inspector CLI; this runs the SDKs themselves the
# way an integrator would write against them: the official TypeScript SDK
# (@modelcontextprotocol/sdk) and the official Python SDK (mcp), each pinned in
# scripts/mcp-sdk-clients/. Each client connects over Streamable HTTP with a
# bearer token, lists the tools, creates a channel and a thread, posts a
# message, reads it back, and checks a bad argument comes back as a tool error.
#
# Boots a source-built Maidan on a throwaway SQLite file with auth ENABLED and
# seeds it with `maidan init`. A harness for local use and a report-only CI job,
# not a required gate.
#
# Usage:  scripts/mcp-sdk-clients.sh [typescript|python ...]   (default: both)
# Env:    MAIDAN_MCP_PORT (default 18091)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
set -euo pipefail

cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

port="${MAIDAN_MCP_PORT:-18091}"
base="http://127.0.0.1:${port}"
bin_dir="${MAIDAN_BIN_DIR:-}"
clients=("$@")
[[ ${#clients[@]} -gt 0 ]] || clients=(typescript python)
here="$(pwd)/scripts/mcp-sdk-clients"

if [[ -z "$bin_dir" ]]; then
  echo "=== building maidan-server + maidan ==="
  cargo build --quiet --bin maidan-server --bin maidan
  bin_dir="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/debug"
fi

work="$(mktemp -d)"
server_pid=""
cleanup() {
  [[ -n "$server_pid" ]] && kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

export DATABASE_URL="sqlite://${work}/maidan.db?mode=rwc"
export MAIDAN_SESSION_SECRET="mcp-sdk-clients-session-secret-0123456789abcdef"

echo "=== server commit $(git rev-parse HEAD 2>/dev/null || echo unknown) ==="
echo "=== seeding (maidan init) ==="
token="$("${bin_dir}/maidan" init --workspace sdk-clients 2>/dev/null | awk '/^    [A-Za-z0-9_-]{20,}$/ {print $1}')"
[[ -n "$token" ]] || { echo "could not read the admin token from maidan init" >&2; exit 1; }

if curl -s -o /dev/null "${base}/health" 2>/dev/null; then
  echo "port ${port} is already serving; set MAIDAN_MCP_PORT" >&2
  exit 1
fi

echo "=== booting (auth enabled) ==="
MAIDAN_BIND="127.0.0.1:${port}" "${bin_dir}/maidan-server" >"${work}/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
  kill -0 "$server_pid" 2>/dev/null || { cat "${work}/server.log" >&2; exit 1; }
  curl -sf "${base}/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "${base}/health" >/dev/null || { cat "${work}/server.log" >&2; exit 1; }

for client in "${clients[@]}"; do
  case "$client" in
    typescript)
      echo "=== @modelcontextprotocol/sdk $(jq -r '.dependencies["@modelcontextprotocol/sdk"]' "${here}/typescript/package.json") ==="
      (cd "${here}/typescript" && npm ci --silent --ignore-scripts)
      for endpoint in /mcp/streamable /mcp; do
        echo "--- ${endpoint}"
        node "${here}/typescript/client.mjs" "$base" "$endpoint" "$token"
      done
      ;;
    python)
      echo "=== $(grep -E '^mcp==' "${here}/python/requirements.txt") ==="
      python3 -m venv "${work}/venv"
      "${work}/venv/bin/pip" install --quiet --disable-pip-version-check -r "${here}/python/requirements.txt"
      for endpoint in /mcp/streamable /mcp; do
        echo "--- ${endpoint}"
        "${work}/venv/bin/python" "${here}/python/client.py" "$base" "$endpoint" "$token"
      done
      ;;
    *)
      echo "unknown client: ${client} (typescript or python)" >&2
      exit 2
      ;;
  esac
done

echo "=== every SDK client check passed ==="
