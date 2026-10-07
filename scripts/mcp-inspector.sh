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
#   2. runs the Inspector CLI against `/mcp` and `/mcp/streamable`;
#   3. makes one valid and one invalid call per tool group, and fails if the
#      invalid call's error does not name the argument it got wrong.
# Exits non-zero on the first failed check.
#
# A reproducible harness for local use and a report-only CI job, not a
# required gate.
#
# Usage:  scripts/mcp-inspector.sh
# Env:    MAIDAN_MCP_PORT (default 18090)
#         MAIDAN_INSPECTOR_VERSION (default 2.7.0, the version verified)
#         MAIDAN_BIN_DIR (default ./target/debug; skip the build if set)
set -euo pipefail

cd "$(dirname "$0")/.."
# A throwaway database, so the public development content KEK is acceptable.
export MAIDAN_ALLOW_INSECURE_DEV_KEK=1

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
# A throwaway export signing seed, so the export group's probe can export.
export MAIDAN_EXPORT_SIGNING_KEY="0101010101010101010101010101010101010101010101010101010101010101"

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

echo "=== one valid and one invalid call per tool group (/mcp/streamable) ==="
endpoint=/mcp/streamable
# `text` is the tool's JSON result, or the error the call printed.
call() {
  local tool="$1"
  shift
  local args=()
  for pair in "$@"; do args+=(--tool-arg "$pair"); done
  run "$endpoint" --method tools/call --tool-name "$tool" ${args[@]+"${args[@]}"} 2>&1
}
field() { jq -r ".content[0].text | fromjson | $1" <<<"$2"; }

me="$(call whoami)" || fail "whoami: ${me}"
ws="$(field .workspace_id "$me")"
member="$(field .member_id "$me")"
made="$(call create_channel "workspace_id=${ws}" name=probes)" || fail "create_channel: ${made}"
channel="$(field .id "$made")"
made="$(call create_thread "channel_id=${channel}" title=probe)" || fail "create_thread: ${made}"
thread="$(field .id "$made")"
made="$(call post_message "thread_id=${thread}" body=probe)" || fail "post_message: ${made}"
message="$(field .id "$made")"
made="$(call request_approval prompt=probe "thread_id=${thread}")" || fail "request_approval: ${made}"
gate="$(field .gate_id "$made")"
made="$(call upload_artifact kind=attachment content_base64=cHJvYmU=)" || fail "upload_artifact: ${made}"
sha="$(field .sha256 "$made")"

# group | a valid call's tool | its arguments | an invalid call's tool | its
# arguments | the argument its error must name. Every module that dispatches a
# tool in crates/maidan-mcp/src/tools/mod.rs has a row, and the check below
# fails when a new module has none. The invalid call must fail and name the
# argument it got wrong, so the error is one an agent can act on. Where a
# group's read takes no argument, the invalid call is one of its writes with a
# required argument missing, which fails before it writes anything. whoami
# takes no argument and its group has no other tool, so it has no invalid call.
probes="
approval|get_approval_gate|gate_id=${gate}|get_approval_gate|gate_id=not-a-uuid|gate_id
artifact|get_artifact_metadata|sha256=${sha}|get_artifact_metadata|sha256=not-a-digest|sha256
automation|list_slash_commands|workspace_id=${ws}|list_slash_commands|workspace_id=not-a-uuid|workspace_id
budget|usage_rollup|workspace_id=${ws}|usage_rollup|workspace_id=not-a-uuid|workspace_id
channel|list_channels|workspace_id=${ws}|list_channels|workspace_id=not-a-uuid|workspace_id
delivery|list_result_deliveries|thread_id=${thread}|list_result_deliveries|thread_id=not-a-uuid|thread_id
event_log|get_log_snapshot|workspace_id=${ws}|get_log_snapshot|workspace_id=not-a-uuid|workspace_id
explorer|list_tombstones|workspace_id=${ws}|list_tombstones|workspace_id=not-a-uuid|workspace_id
export|export_workspace|workspace_id=${ws}|export_workspace|workspace_id=not-a-uuid|workspace_id
freeze|list_frozen_members||freeze_member|reason=probe|member_id
glossary|list_glossary_terms||get_glossary_term||term
land_gate|get_land_gate|thread_id=${thread}|get_land_gate|thread_id=not-a-uuid|thread_id
member|list_members|workspace_id=${ws}|list_members|workspace_id=not-a-uuid|workspace_id
memory_block|list_memory_blocks||get_memory_block||label
message|list_messages|thread_id=${thread}|list_messages|thread_id=not-a-uuid|thread_id
projector|list_github_issue_links||link_github_issue|repo=o/r issue_number=1|thread_id
recipe|list_recipes||create_recipe|name=probe spec={}|channel_id
reference|list_references|src_kind=thread src_id=${thread}|list_references|src_id=not-a-uuid|src_id
review|get_review_status|thread_id=${thread}|get_review_status|thread_id=not-a-uuid|thread_id
room|get_room|workspace_id=${ws}|get_room|workspace_id=not-a-uuid|workspace_id
schedule|list_task_schedules||create_task_schedule|title=probe|channel_id
secret|list_secrets||create_secret|value=probe|name
seed|seed_from_message|message_id=${message} title=seeded|seed_from_message|message_id=not-a-uuid title=seeded|message_id
share|list_share_tickets||create_share_ticket|expires_at=2030-01-01T00:00:00Z|channel_id
skill|list_member_skills|member_id=${member}|list_member_skills|member_id=not-a-uuid|member_id
snapshot|snapshot_thread_context|thread_id=${thread}|snapshot_thread_context|thread_id=not-a-uuid|thread_id
social|list_pins|thread_id=${thread}|list_pins|thread_id=not-a-uuid|thread_id
spawn|get_spawn_budget||set_spawn_budget|max_depth=-1|max_depth
thread|get_thread_context|thread_id=${thread}|get_thread_context|thread_id=not-a-uuid|thread_id
whoami|whoami||||
"

modules="$(sed -nE 's/.*"[a-z_]+" => ([a-z_]+)::.*/\1/p' crates/maidan-mcp/src/tools/mod.rs | sort -u)"
probed="$(awk -F'|' 'NF > 1 {print $1}' <<<"$probes" | sort -u)"
missing="$(comm -23 <(echo "$modules") <(echo "$probed"))"
[[ -z "$missing" ]] || fail "tool groups with no probe row: ${missing//$'\n'/ }"

ok() { jq -e '.isError != true' <<<"$1" >/dev/null 2>&1; }
while IFS='|' read -r group tool valid bad_tool bad named; do
  [[ -n "$group" ]] || continue
  # shellcheck disable=SC2086 # a row's arguments are space-separated pairs
  if ! out="$(call "$tool" $valid)" || ! ok "$out"; then
    fail "${group}: ${tool} ${valid}: ${out}"
  fi
  if [[ -n "$bad_tool" ]]; then
    # shellcheck disable=SC2086
    if out="$(call "$bad_tool" $bad)" && ok "$out"; then
      fail "${group}: ${bad_tool} ${bad} succeeded: ${out}"
    fi
    grep -q "$named" <<<"$out" || fail "${group}: ${bad_tool} ${bad} failed without naming ${named}: ${out}"
  fi
  pass "${group}: ${tool}${bad_tool:+, and ${bad_tool} refused naming ${named}}"
done <<<"$probes"

echo "=== all MCP external checks passed ==="
