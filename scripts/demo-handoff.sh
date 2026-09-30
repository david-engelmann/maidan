#!/usr/bin/env bash
# The README's terminal recording: two agents hand off a task and a human
# signs it off, against a real server.
#
#   planner (agent)  opens a task thread in #build and briefs it (REST + MCP)
#   coder   (agent)  claims it with claim_next_thread, reports, sets a result
#   david   (human)  reads the thread back and closes it
#
# Every line printed is the server's own answer; ids and times are real. The
# members, channel and task text are demo data. Needs the same two values the
# quickstart prints (`maidan init`), plus curl and jq:
#
#   ./scripts/demo-handoff.sh
#
# With MAIDAN_TOKEN and MAIDAN_WORKSPACE unset, the script runs `maidan init`
# against DATABASE_URL and then creates the planner, the coder, and the human,
# with tokens that can claim, post, transition, and review.
#
#   MAIDAN_TOKEN=<admin token> MAIDAN_WORKSPACE=<workspace id> ./scripts/demo-handoff.sh
#
# DEMO_PAUSE (seconds, default 0) spaces the steps out for a recording.
set -euo pipefail

BASE="${MAIDAN_URL:-http://127.0.0.1:8080}"
PAUSE="${DEMO_PAUSE:-0}"
SUFFIX="${DEMO_SUFFIX:-}"

# The script sends bearer tokens on every call: plain http only to loopback.
if [[ "$BASE" == http://* ]]; then
  authority="${BASE#http://}"; authority="${authority%%/*}"
  case "$authority" in
    *@*) host= ;;                                   # userinfo hides the real host
    "[::1]" | "[::1]:"*) host="[::1]" ;;
    *) host="${authority%%:*}" ;;
  esac
  case "$host" in
    127.0.0.1 | localhost | "[::1]") ;;
    *) echo "refusing to send tokens to $BASE over plain http; use https or a loopback address" >&2; exit 1 ;;
  esac
elif [[ "$BASE" != https://* ]]; then
  echo "MAIDAN_URL must be an http(s) URL, got $BASE" >&2; exit 1
fi

for cmd in curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

if [[ -n "${MAIDAN_TOKEN:-}" && -n "${MAIDAN_WORKSPACE:-}" ]]; then
  ADMIN="$MAIDAN_TOKEN"
  WS="$MAIDAN_WORKSPACE"
else
  command -v maidan >/dev/null 2>&1 || { echo "set MAIDAN_TOKEN and MAIDAN_WORKSPACE, or install maidan so this script can create the workspace" >&2; exit 1; }
  case "${DATABASE_URL:-}" in
    ""|sqlite::memory:*) echo "set DATABASE_URL to the server's database (or pass MAIDAN_TOKEN and MAIDAN_WORKSPACE)" >&2; exit 1 ;;
  esac
  init_out=$(maidan init --workspace "${MAIDAN_WORKSPACE_NAME:-demo}" --admin-handle admin)
  WS=$(sed -n 's/^  workspace: .* (\(.*\))$/\1/p' <<<"$init_out")
  ADMIN=$(awk '/Admin bearer token/{show=1; next} show && $0 ~ /^[[:space:]]*$/ {next} show {gsub(/^[[:space:]]+|[[:space:]]+$/, ""); print; exit}' <<<"$init_out")
  if [[ -z "$ADMIN" || -z "$WS" ]]; then
    echo "could not read the workspace and admin token from maidan init" >&2
    printf '%s\n' "$init_out" >&2
    exit 1
  fi
fi

if [ -t 1 ]; then B=$'\e[1m'; D=$'\e[90m'; G=$'\e[32m'; X=$'\e[31m'; C=$'\e[36m'; Y=$'\e[33m'; M=$'\e[35m'; R=$'\e[0m'
else B=; D=; G=; X=; C=; Y=; M=; R=; fi

rest() { # rest <token> <METHOD> <path> [json]
  curl -fsS -X "$2" -H "authorization: Bearer $1" -H 'content-type: application/json' \
    "$BASE$3" ${4:+--data "$4"}
}
mcp() { # mcp <token> <tool> <json-args>  -> the tool's JSON result
  local out
  out=$(curl -fsS -H "authorization: Bearer $1" -H 'content-type: application/json' "$BASE/mcp" \
    --data "$(jq -nc --arg n "$2" --argjson a "$3" \
      '{jsonrpc:"2.0",id:1,method:"tools/call",params:{name:$n,arguments:$a}}')")
  if [ "$(jq -r '.result.isError // false' <<<"$out")" = true ] || jq -e '.error' <<<"$out" >/dev/null; then
    echo "MCP $2 failed: $out" >&2; exit 1
  fi
  jq -r '.result.content[0].text' <<<"$out"
}
mcp_refused() { # mcp_refused <token> <tool> <json-args> <expected>  -> the refusal text; fails unless refused for <expected>
  local out
  out=$(curl -fsS -H "authorization: Bearer $1" -H 'content-type: application/json' "$BASE/mcp" \
    --data "$(jq -nc --arg n "$2" --argjson a "$3" \
      '{jsonrpc:"2.0",id:1,method:"tools/call",params:{name:$n,arguments:$a}}')")
  if [ "$(jq -r '.result.isError // false' <<<"$out")" != true ] && ! jq -e '.error' <<<"$out" >/dev/null; then
    echo "MCP $2 was expected to be refused: $out" >&2; exit 1
  fi
  local why
  why=$(jq -r '.error.message // .result.content[0].text' <<<"$out")
  case "$why" in *"$4"*) printf '%s\n' "$why" ;; *) echo "MCP $2 was refused for another reason: $why" >&2; exit 1 ;; esac
}
step() { sleep "$PAUSE"; printf '\n%s%s%s %s▸%s %s\n' "$1" "$2" "$R" "$D" "$R" "$3"; }
say()  { printf '  %s\n' "$*"; }

# Setup (not shown in the story): three members with scoped tokens, one channel.
member() { rest "$ADMIN" POST "/workspaces/$WS/members" "{\"handle\":\"$1$SUFFIX\",\"kind\":\"$2\"}" | jq -r .id; }
token()  { rest "$ADMIN" POST "/workspaces/$WS/members/$1/tokens" "{\"capabilities\":$2}" | jq -r .secret; }
agent_caps='["workspace:read","workspace:write","message:post","thread:transition"]'
human_caps='["workspace:read","workspace:write","message:post","thread:transition","event:subscribe"]'
planner=$(member planner agent); coder=$(member coder agent); david=$(member david human)
PT=$(token "$planner" "$agent_caps"); CT=$(token "$coder" "$agent_caps"); HT=$(token "$david" "$human_caps")
channel=$(rest "$ADMIN" POST "/workspaces/$WS/channels" "{\"name\":\"build$SUFFIX\"}" | jq -r .id)
name() { case "$1" in "$planner") echo planner;; "$coder") echo coder;; "$david") echo david;; *) echo "$1";; esac; }

printf '%sMaidan%s %s%s · workspace %s · demo data%s\n' "$B" "$R" "$D" "$BASE" "${WS:0:8}…" "$R"

step "$C" "planner" "REST POST /channels/#build/threads   (open a task)"
thread=$(rest "$PT" POST "/channels/$channel/threads" '{"title":"Fix the flaky login test"}')
tid=$(jq -r .id <<<"$thread")
say "${G}✓${R} task ${B}$(jq -r .title <<<"$thread")${R}  state=$(jq -r .state <<<"$thread")"
mcp "$PT" post_message "{\"thread_id\":\"$tid\",\"body\":\"login_e2e fails 1 run in 20 on CI. Find the race and fix it.\"}" >/dev/null
say "${G}✓${R} MCP post_message: \"login_e2e fails 1 run in 20 on CI. Find the race…\""
mcp "$PT" set_review_requirement "{\"thread_id\":\"$tid\",\"required_count\":1}" >/dev/null
mcp "$PT" add_reviewer "{\"thread_id\":\"$tid\",\"member_id\":\"$david\"}" >/dev/null
say "${G}✓${R} MCP set_review_requirement 1 · add_reviewer david"

step "$Y" "coder" "MCP claim_next_thread {channel: #build, lease_secs: 900}"
claim=$(mcp "$CT" claim_next_thread "{\"channel_id\":\"$channel\",\"lease_secs\":900}")
if ! jq -e --arg tid "$tid" '. != null and .id == $tid' <<<"$claim" >/dev/null; then
  echo "claim_next_thread did not hand the coder thread $tid: $claim" >&2; exit 1
fi
say "${G}✓${R} claimed ${B}$(jq -r .title <<<"$claim")${R}  assignee=$(name "$(jq -r .assignee_id <<<"$claim")")  lease until $(jq -r '.assignment_expires_at[11:19]' <<<"$claim")Z"
rival=$(mcp "$PT" claim_next_thread "{\"channel_id\":\"$channel\"}")
say "${D}·${R} planner tries claim_next_thread too → ${rival}  ${D}(one holder at a time)${R}"

step "$Y" "coder" "MCP post_message · set_thread_result · transition_thread start_review"
mcp "$CT" post_message "{\"thread_id\":\"$tid\",\"body\":\"Session save wasn't awaited before redirect. Fixed; 500/500 green.\"}" >/dev/null
result=$(mcp "$CT" set_thread_result "{\"thread_id\":\"$tid\",\"result\":{\"status\":\"fixed\",\"runs\":500,\"failures\":0}}")
review=$(mcp "$CT" transition_thread "{\"thread_id\":\"$tid\",\"action\":\"start_review\"}")
say "${G}✓${R} result $(jq -c .result <<<"$result")  state=$(jq -r .state <<<"$review")"
refusal=$(mcp_refused "$CT" transition_thread "{\"thread_id\":\"$tid\",\"action\":\"close\"}" "review requirement not met")
say "${X}✗${R} coder tries to close it → ${X}refused${R}:"
say "  ${D}${refusal#invalid params: }${R}"

step "$M" "david" "REST GET /threads/:id/context   (human reads the thread)"
ctx=$(rest "$HT" GET "/threads/$tid/context")
while IFS=$'\t' read -r author body; do
  say "$(printf '%-9s' "$(name "$author"):")${body}"
done < <(jq -r '.messages[] | [.author_id, .body] | @tsv' <<<"$ctx")
res=$(rest "$HT" GET "/threads/$tid/result")
say "$(printf '%-9s' "result:")$(jq -c .result <<<"$res") by $(name "$(jq -r .produced_by <<<"$res")")"

step "$M" "david" "MCP submit_review approve · REST POST /threads/:id {action: close}"
mcp "$HT" submit_review "{\"thread_id\":\"$tid\",\"decision\":\"approve\"}" >/dev/null
closed=$(rest "$HT" POST "/threads/$tid" '{"action":"close"}')
say "${G}✓${R} approved · state=$(jq -r .state <<<"$closed")"

step "$G" "log" "MCP verify_event_chain"
chain=$(mcp "$ADMIN" verify_event_chain '{}')
ok=$(jq -r .ok <<<"$chain")
say "${G}✓${R} $(jq -r .checked <<<"$chain") events, $(jq -r .algorithm <<<"$chain") hash chain intact=$ok  head=$(jq -r '.head.content_hash[7:19]' <<<"$chain")…"
if [ "$ok" != "true" ]; then
  echo "event chain did not verify: $chain" >&2
  exit 1
fi
sleep "$PAUSE"
