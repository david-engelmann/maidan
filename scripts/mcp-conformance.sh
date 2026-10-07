#!/usr/bin/env bash
# The official MCP conformance suite against a real Maidan.
#
# It runs the suite's frozen requirement set for each revision Maidan
# negotiates on the stateless transport, 2025-11-25 and 2026-07-28. That needs
# the suite's 0.2 line: the stable 0.1 release has no 2026-07-28 scenarios and
# baselines only whole scenarios, while 0.2 baselines single checks, so one
# known gap does not excuse the rest of its scenario.
#
# The suite's server mode sends no credential, so it runs against a dev
# instance in the anonymous read-only mode (MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE)
# on a throwaway synthetic workspace. Each known failure is listed with its
# reason in scripts/mcp-conformance/<revision>.yml. A run fails on a check
# that newly fails and on a listed one that now passes, so the lists stay true.
#
# A reproducible harness for local use and a report-only CI job, not a
# required gate.
#
# Usage:  scripts/mcp-conformance.sh
# Env:    MAIDAN_MCP_PORT (default 18091)
#         MAIDAN_CONFORMANCE_VERSION (default 0.2.0-alpha.11, the version baselined)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
#         MAIDAN_CONFORMANCE_OUT (keep the results in this directory)
set -euo pipefail

cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

port="${MAIDAN_MCP_PORT:-18091}"
suite="@modelcontextprotocol/conformance@${MAIDAN_CONFORMANCE_VERSION:-0.2.0-alpha.11}"
base="http://127.0.0.1:${port}"
bin_dir="${MAIDAN_BIN_DIR:-}"
out="${MAIDAN_CONFORMANCE_OUT:-}"
# The binaries refuse a MAIDAN_* variable they do not know, so this script's
# own settings stop here.
unset MAIDAN_MCP_PORT MAIDAN_CONFORMANCE_VERSION MAIDAN_BIN_DIR MAIDAN_CONFORMANCE_OUT

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
export MAIDAN_SESSION_SECRET="mcp-conformance-session-secret-0123456789abcdef"

echo "=== seeding a synthetic workspace (maidan init) ==="
workspace="$("${bin_dir}/maidan" init --workspace synthetic-conformance 2>/dev/null |
  sed -nE 's/^  workspace: synthetic-conformance +\(([0-9a-f-]{36})\)$/\1/p')"
[[ -n "$workspace" ]] || { echo "could not read the workspace id from maidan init" >&2; exit 1; }

if curl -s -o /dev/null "${base}/health" 2>/dev/null; then
  echo "port ${port} is already serving; set MAIDAN_MCP_PORT" >&2
  exit 1
fi

echo "=== booting (auth enabled, anonymous reads of ${workspace}) ==="
MAIDAN_BIND="127.0.0.1:${port}" MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE="$workspace" \
  "${bin_dir}/maidan-server" >"${work}/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
  kill -0 "$server_pid" 2>/dev/null || { cat "${work}/server.log" >&2; exit 1; }
  curl -sf "${base}/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -sf "${base}/health" >/dev/null || { cat "${work}/server.log" >&2; exit 1; }

out="${out:-${work}/results}"
status=0
for revision in 2025-11-25 2026-07-28; do
  echo "=== ${suite}: the ${revision} requirements against /mcp/streamable ==="
  npx -y "$suite" server --url "${base}/mcp/streamable" --requirements "$revision" \
    --expected-failures "scripts/mcp-conformance/${revision}.yml" \
    --output-dir "${out}/${revision}" || status=1
done
exit "$status"
