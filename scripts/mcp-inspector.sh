#!/usr/bin/env bash
# MCP external verifier: the official MCP Inspector against a real Maidan.
#
# Maidan's own tests prove its MCP server agrees with itself. This proves it
# agrees with the client most people will point at it: the official Inspector,
# built on the official TypeScript SDK, run as an unmodified external process.
# It is how we found that the SDK could not complete a handshake at all.
#
# Boots a source-built Maidan on a throwaway SQLite file with auth ENABLED,
# seeds it with `maidan init` (a real bearer token), then:
#   1. checks `initialize` negotiation for every supported revision directly;
#   2. runs the Inspector CLI against `/mcp` and `/mcp/streamable`.
# Exits non-zero on the first failed check.
#
# Like scripts/a2a-interop.sh this is a reproducible harness for local use and
# a report-only CI job, not a required gate.
#
# Usage:  scripts/mcp-inspector.sh
# Env:    MAIDAN_MCP_PORT (default 18090)
#         MAIDAN_INSPECTOR_VERSION (default 2.7.0, the version verified)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
set -euo pipefail

cd "$(dirname "$0")/.."

port="${MAIDAN_MCP_PORT:-18090}"
inspector="@modelcontextprotocol/inspector@${MAIDAN_INSPECTOR_VERSION:-2.7.0}"
base="http://127.0.0.1:${port}"
bin_dir="${MAIDAN_BIN_DIR:-}"

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
export MAIDAN_SESSION_SECRET="mcp-inspector-session-secret-0123456789abcdef"

echo "=== seeding (maidan init) ==="
token="$("${bin_dir}/maidan" init --workspace inspector 2>/dev/null | awk '/^    [A-Za-z0-9_-]{20,}$/ {print $1}')"
[[ -n "$token" ]] || { echo "could not read the admin token from maidan init" >&2; exit 1; }

# A health check answered by whatever else holds the port would pass every
# later check against the wrong server, so refuse a port that is already taken.
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

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
pass() { echo "  ok  $*"; }

echo "=== initialize negotiation, every revision ==="
for version in 2026-07-28 2025-11-25 2025-06-18 2025-03-26; do
  headers="${work}/h"
  body="$(curl -s -D "$headers" -X POST "${base}/mcp/streamable" \
    -H "Authorization: Bearer ${token}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"${version}\",\"capabilities\":{},\"clientInfo\":{\"name\":\"verifier\",\"version\":\"0\"}}}")"
  got="$(jq -r '.result.protocolVersion // empty' <<<"$body")"
  [[ "$got" == "$version" ]] || fail "initialize ${version} negotiated '${got}': ${body}"
  grep -qi '^mcp-session-id:' "$headers" && fail "initialize ${version} minted a session; it is stateless"
  pass "${version}: echoed, stateless"
done
unknown="$(curl -s -X POST "${base}/mcp/streamable" -H "Authorization: Bearer ${token}" \
  -H 'Content-Type: application/json' -H 'Accept: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}' |
  jq -r '.result.protocolVersion')"
[[ "$unknown" == "2026-07-28" ]] || fail "an unknown revision should get the latest, got '${unknown}'"
pass "unknown revision: offered 2026-07-28"

run() {
  local endpoint="$1"
  shift
  npx -y "$inspector" --cli "${base}${endpoint}" --transport http \
    --header "Authorization: Bearer ${token}" "$@"
}

for endpoint in /mcp/streamable /mcp; do
  echo "=== Inspector ${inspector} against ${endpoint} ==="

  tools="$(run "$endpoint" --method tools/list)" || fail "tools/list: ${tools}"
  count="$(jq '.tools | length' <<<"$tools")"
  (( count > 100 )) || fail "tools/list returned ${count} tools"
  pass "tools/list: ${count} tools, every schema accepted by the SDK"

  whoami="$(run "$endpoint" --method tools/call --tool-name whoami)" || fail "whoami: ${whoami}"
  jq -e '.content[0].text | fromjson | .member_id and (.bypass | not)' <<<"$whoami" >/dev/null ||
    fail "whoami did not return a bearer identity: ${whoami}"
  pass "tools/call whoami: authenticated as the bearer"

  resources="$(run "$endpoint" --method resources/list)" || fail "resources/list: ${resources}"
  jq -e '.resources | length > 0 and all(.uri | test("[{}]") | not)' <<<"$resources" >/dev/null ||
    fail "resources/list must list concrete URIs: ${resources}"
  workspace_uri="$(jq -r '.resources[0].uri' <<<"$resources")"
  pass "resources/list: concrete (${workspace_uri})"

  read="$(run "$endpoint" --method resources/read --uri "$workspace_uri")" || fail "resources/read: ${read}"
  jq -e '.contents | length > 0' <<<"$read" >/dev/null || fail "resources/read returned nothing: ${read}"
  pass "resources/read: the listed resource is readable"

  templates="$(run "$endpoint" --method resources/templates/list)" || fail "resources/templates/list: ${templates}"
  jq -e '.resourceTemplates | length >= 4 and all(.uriTemplate | test("[{]"))' <<<"$templates" >/dev/null ||
    fail "resources/templates/list: ${templates}"
  pass "resources/templates/list: $(jq '.resourceTemplates | length' <<<"$templates") templates"

  prompts="$(run "$endpoint" --method prompts/list)" || fail "prompts/list: ${prompts}"
  jq -e '.prompts | length > 0' <<<"$prompts" >/dev/null || fail "prompts/list: ${prompts}"
  pass "prompts/list: $(jq '.prompts | length' <<<"$prompts") prompts"
done

echo "=== all MCP external checks passed ==="
