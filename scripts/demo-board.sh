#!/usr/bin/env bash
# The README's hero: several agents and one human share a #build board.
#
#   seed   four agents and a human, a #build channel, and a backlog of tasks in
#          every state: done, in review, being worked, claimed, open
#   live   the story the hero shows, one step per line: a planner files a task,
#          a coder hands its fix to review and is refused when it tries to close
#          its own work, two coders ask for the next task at the same moment and
#          get different ones, and the human approves and closes
#
# Every call is real (REST and MCP against a running server) and every line
# printed is built from the server's answer. The members, channel and task text
# are demo data. Needs what `maidan init` prints, plus curl and jq:
#
#   ./scripts/demo-board.sh
#
# With MAIDAN_TOKEN and MAIDAN_WORKSPACE unset, the script runs `maidan init`
# against DATABASE_URL (the same database the server is using) and reads the
# workspace id and admin token from that output. It then creates the human,
# every agent, and tokens that can claim, post, transition, and review.
# Nothing here asks you to create a member or pick capabilities.
#
#   MAIDAN_TOKEN=… MAIDAN_WORKSPACE=… ./scripts/demo-board.sh
#
# DEMO_PAUSE (seconds, default 0) spaces the live steps out for a recording;
# DEMO_GO_FILE, when set, holds the live steps until that file exists.
# The human's token (workspace:read + event:subscribe, for /ui) is written to
# DEMO_VIEWER_TOKEN_FILE when that is set.
# DEMO_HUMAN_DONE_FILE, when set, leaves the human's approve and close to a
# person (or a recorder) clicking Approve and Close task in /ui's Needs you
# row: the script waits for that file, then prints the state the server
# reports for the task.
set -euo pipefail

BASE="${MAIDAN_URL:-http://127.0.0.1:8080}"
PAUSE="${DEMO_PAUSE:-0}"

if [[ "$BASE" == http://* ]]; then
  authority="${BASE#http://}"; authority="${authority%%/*}"
  case "$authority" in
    *@*) host= ;;
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

# Workspace, admin token, then the cast. Init is only used when the caller
# did not already pass what `maidan init` printed.
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
  echo "created workspace $WS" >&2
fi

rest() { # rest <token> <METHOD> <path> [json]
  curl -fsS -X "$2" -H "authorization: Bearer $1" -H 'content-type: application/json' \
    "$BASE$3" ${4:+--data "$4"}
}
mcp_raw() {
  curl -fsS -H "authorization: Bearer $1" -H 'content-type: application/json' "$BASE/mcp" \
    --data "$(jq -nc --arg n "$2" --argjson a "$3" \
      '{jsonrpc:"2.0",id:1,method:"tools/call",params:{name:$n,arguments:$a}}')"
}
mcp() { # mcp <token> <tool> <json-args> -> the tool's JSON result
  local out; out=$(mcp_raw "$@")
  if [ "$(jq -r '.result.isError // false' <<<"$out")" = true ] || jq -e '.error' <<<"$out" >/dev/null; then
    echo "MCP $2 failed: $out" >&2; exit 1
  fi
  jq -r '.result.content[0].text' <<<"$out"
}
mcp_refused() { # mcp_refused <token> <tool> <json-args> <expected> -> refusal text
  local out why; out=$(mcp_raw "$1" "$2" "$3")
  if [ "$(jq -r '.result.isError // false' <<<"$out")" != true ] && ! jq -e '.error' <<<"$out" >/dev/null; then
    echo "MCP $2 was expected to be refused: $out" >&2; exit 1
  fi
  why=$(jq -r '.error.message // .result.content[0].text' <<<"$out")
  case "$why" in *"$4"*) printf '%s\n' "${why#invalid params: }" ;; *) echo "MCP $2 refused for another reason: $why" >&2; exit 1 ;; esac
}

member() { rest "$ADMIN" POST "/workspaces/$WS/members" \
  "$(jq -nc --arg h "$1" --arg d "$2" --arg k "$3" '{handle:$h,display_name:$d,kind:$k}')" | jq -r .id; }
token()  { rest "$ADMIN" POST "/workspaces/$WS/members/$1/tokens" "{\"capabilities\":$2}" | jq -r .secret; }
agent_caps='["workspace:read","workspace:write","message:post","thread:transition"]'
human_caps='["workspace:read","message:post","thread:transition","event:subscribe"]'

planner=$(member planner Planner agent)
coder_a=$(member coder-a "Coder A" agent)
coder_b=$(member coder-b "Coder B" agent)
tester=$(member tester Tester agent)
david=$(member david David human)
PT=$(token "$planner" "$agent_caps"); AT=$(token "$coder_a" "$agent_caps")
BT=$(token "$coder_b" "$agent_caps"); TT=$(token "$tester" "$agent_caps")
HT=$(token "$david" "$human_caps")
[ -n "${DEMO_VIEWER_TOKEN_FILE:-}" ] && printf '%s\n' "$HT" > "$DEMO_VIEWER_TOKEN_FILE"
channel=$(rest "$ADMIN" POST "/workspaces/$WS/channels" '{"name":"build","topic":"demo data"}' | jq -r .id)
[ -n "${DEMO_CHANNEL_FILE:-}" ] && printf '%s\n' "$channel" > "$DEMO_CHANNEL_FILE"

task() { # task <title> <brief> -> thread id, filed by the planner
  local tid
  tid=$(rest "$PT" POST "/channels/$channel/threads" "$(jq -nc --arg t "$1" '{title:$t}')" | jq -r .id)
  mcp "$PT" post_message "$(jq -nc --arg t "$tid" --arg b "$2" '{thread_id:$t,body:$b}')" >/dev/null
  printf '%s\n' "$tid"
}
claim() { # claim <token> -> the claimed thread JSON (oldest open task)
  mcp "$1" claim_next_thread "{\"channel_id\":\"$channel\",\"lease_secs\":900}"
}
start_work() { # start_work <token> <claim json>
  rest "$1" POST "/threads/$(jq -r .id <<<"$2")/claim/acknowledge" \
    "$(jq -c '{claim_lease_id}' <<<"$2")" >/dev/null
}
report() { # report <token> <tid> <message> <result json>
  mcp "$1" post_message "$(jq -nc --arg t "$2" --arg b "$3" '{thread_id:$t,body:$b}')" >/dev/null
  mcp "$1" set_thread_result "$(jq -nc --arg t "$2" --argjson r "$4" '{thread_id:$t,result:$r}')" >/dev/null
  mcp "$1" transition_thread "{\"thread_id\":\"$2\",\"action\":\"start_review\"}" | jq -r .state
}
approve_close() { # approve_close <tid>
  mcp "$HT" submit_review "{\"thread_id\":\"$1\",\"decision\":\"approve\"}" >/dev/null
  rest "$HT" POST "/threads/$1" '{"action":"close"}' | jq -r .state
}
gate() { # one human approval before close. The reviewer is named only when asked.
  mcp "$PT" set_review_requirement "{\"thread_id\":\"$1\",\"required_count\":1}" >/dev/null
}
needs_you() { # this task is the one waiting on the human
  gate "$1"
  mcp "$PT" add_reviewer "{\"thread_id\":\"$1\",\"member_id\":\"$david\"}" >/dev/null
}

# ---- seed: a backlog in every state ------------------------------------
t_sqlx=$(task "Bump sqlx to 0.8" "Bump sqlx to 0.8 and fix whatever breaks.")
t_reqid=$(task "Add request IDs to access logs" "Every access-log line should carry the request id.")
t_login=$(task "Fix the flaky login test" "login_e2e fails 1 run in 20 on CI. Find the race and fix it.")
t_rate=$(task "Rate-limit /api/upload" "Cap uploads at 10/min per token; 429 with Retry-After.")
t_load=$(task "Load-test the upload path" "p99 under 200 ms at 50 rps, or tell me where it breaks.")
t_notes=$(task "Write upgrade notes for 2.0" "Breaking changes, in the order an operator hits them.")
t_cache=$(task "Cache the OpenAPI document" "Serve /openapi.json from memory; it never changes at runtime.")
# David is the reviewer of the login task only. The rate-limit card can sit
# in review without joining Needs you, so the queue has one obvious row.
for t in "$t_sqlx" "$t_reqid" "$t_rate" "$t_load" "$t_notes" "$t_cache"; do gate "$t"; done
needs_you "$t_login"

c=$(claim "$BT"); start_work "$BT" "$c"
report "$BT" "$t_sqlx" "Bumped; two query macros needed explicit types. CI green." '{"status":"done","pr":"#412"}' >/dev/null
approve_close "$t_sqlx" >/dev/null
c=$(claim "$AT"); start_work "$AT" "$c"
report "$AT" "$t_reqid" "Request id is now on every access-log line." '{"status":"done","pr":"#415"}' >/dev/null
approve_close "$t_reqid" >/dev/null
c=$(claim "$AT"); start_work "$AT" "$c"          # login test: Coder A is on it
mcp "$AT" post_message "$(jq -nc --arg t "$t_login" '{thread_id:$t,body:"Reproduced: 1 in 20 locally with --repeat 200. Looking at the session save."}')" >/dev/null
c=$(claim "$BT"); start_work "$BT" "$c"          # rate limit: Coder B, now waiting on review
report "$BT" "$t_rate" "10/min per token, 429 with Retry-After. Tests cover the burst edge." '{"status":"done","limit_per_min":10}' >/dev/null
c=$(claim "$TT"); start_work "$TT" "$c"          # load test: Tester is on it
echo "seeded: 5 members, #build, 7 tasks" >&2
[ "${1:-}" = "seed" ] && exit 0
# A recorder can hold the story until its browser is on the board.
if [ -n "${DEMO_GO_FILE:-}" ]; then while [ ! -e "$DEMO_GO_FILE" ]; do sleep 0.1; done; fi

# ---- live: the story the hero shows ------------------------------------
line() { printf '%s\t%s\n' "$1" "$2"; }
sleep "$PAUSE"
t_retry=$(task "Retry webhook deliveries with backoff" "Retry 5xx deliveries with jittered backoff, max 6 tries.")
needs_you "$t_retry"
line planner "REST POST /channels/#build/threads → open: Retry webhook deliveries with backoff"
sleep "$PAUSE"
state=$(report "$AT" "$t_login" "Race: session save wasn't awaited before redirect. Fixed; 500/500 green." \
  '{"status":"fixed","runs":500,"failures":0}')
line coder-a "MCP set_thread_result {runs: 500, failures: 0} · transition_thread start_review → $state"
sleep "$PAUSE"
why=$(mcp_refused "$AT" transition_thread "{\"thread_id\":\"$t_login\",\"action\":\"close\"}" "review requirement not met")
line refused "coder-a MCP transition_thread close → refused: $why"
sleep "$PAUSE"
# Two agents ask for the next task at the same moment; each gets a different one.
claim "$AT" > /tmp/demo-board-claim-a.$$ & claim "$BT" > /tmp/demo-board-claim-b.$$ & wait
ca=$(cat /tmp/demo-board-claim-a.$$); cb=$(cat /tmp/demo-board-claim-b.$$); rm -f /tmp/demo-board-claim-[ab].$$
ta=$(jq -r '.title // empty' <<<"$ca"); tb=$(jq -r '.title // empty' <<<"$cb")
if [ -z "$ta" ] || [ -z "$tb" ] || [ "$ta" = "$tb" ]; then
  echo "the two claims were not two different tasks: $ca | $cb" >&2
  exit 1
fi
line coder-a "MCP claim_next_thread → $ta"
line coder-b "MCP claim_next_thread → $tb   (same moment, different task)"
sleep "$PAUSE"
# One obvious Needs-you row: the login task, and nothing else named for David.
david_id=$(mcp "$HT" whoami '{}' | jq -r .member_id)
inbox=$(rest "$HT" GET "/members/$david_id/waiting")
rows=$(jq -c '[.items[] | select(.kind=="review_request")]' <<<"$inbox")
count=$(jq 'length' <<<"$rows")
title=$(jq -r '.[0].summary // empty' <<<"$rows")
if [ "$count" != 1 ] || [[ "$title" != *"Fix the flaky login test"* ]]; then
  echo "Needs you should be the login task alone, got $inbox" >&2
  exit 1
fi
line needs "Needs you: 1 · $title"
if [ -n "${DEMO_HUMAN_DONE_FILE:-}" ]; then
  line prompt "david: Approve, then Close task, in the Needs you row"
  while [ ! -e "$DEMO_HUMAN_DONE_FILE" ]; do sleep 0.1; done
  state=$(rest "$HT" GET "/threads/$t_login" | jq -r .state)
  line david "/ui Needs you: Approve · Close task → $state: Fix the flaky login test"
else
  mcp "$HT" post_message "$(jq -nc --arg t "$t_login" '{thread_id:$t,body:"Checked the diff and the 500-run log. Approving."}')" >/dev/null
  state=$(approve_close "$t_login")
  line david "MCP submit_review approve · close → $state: Fix the flaky login test"
fi
if [ "$state" != "closed" ]; then
  echo "the login task was not closed (state=$state)" >&2
  exit 1
fi
sleep "$PAUSE"
chain=$(mcp "$ADMIN" verify_event_chain '{}')
line chain "MCP verify_event_chain → checked=$(jq -r .checked <<<"$chain") algorithm=$(jq -r .algorithm <<<"$chain") ok=$(jq -r .ok <<<"$chain") head=$(jq -r .head.content_hash <<<"$chain")"
if [ "$(jq -r .ok <<<"$chain")" != "true" ]; then
  echo "event chain did not verify: $chain" >&2
  exit 1
fi
