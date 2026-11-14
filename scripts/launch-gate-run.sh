#!/usr/bin/env bash
# The terminal half of docs/Launch Gate.md, the maidan-launch-1.0 gate run
# against maidan.world. It makes every operator and agent call in the
# runbook's order, finds the ids itself, checks what a terminal can check, and
# keeps the evidence. It stops at each browser step and says what to do there.
#
#   scripts/launch-gate-run.sh [EVIDENCE_DIR]
#
# EVIDENCE_DIR defaults to ~/launch-gate-YYYYMMDD (UTC) and must be outside
# the repository. ORIGIN overrides https://maidan.world, and EXPECT_VERSION
# names the release under test (default: whatever /health reports).
#
# Tokens are read without echo and never written anywhere. The invite's
# signup_path is shown on the terminal, once, for person A or B, and is never
# saved. Screenshots stay yours to take; the record lists each one.
set -euo pipefail

ORIGIN="${ORIGIN:-https://maidan.world}"
EV="${1:-$HOME/launch-gate-$(date -u +%Y%m%d)}"
STAMP="$(date -u +%Y%m%d)"
AGENT_CALLS=0

for tool in curl jq uuidgen; do
  command -v "$tool" >/dev/null || { echo "needs $tool" >&2; exit 2; }
done
case "$(cd "$(dirname "$EV")" 2>/dev/null && pwd)/" in
  "$(git rev-parse --show-toplevel 2>/dev/null || echo /nonexistent)"/*)
    echo "keep the evidence outside the repository: $EV" >&2
    exit 2
    ;;
esac
mkdir -p "$EV"
chmod 700 "$EV"
: >"$EV/checks.txt"

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
browser() {
  printf '\n\033[36m[browser] %s\033[0m\n' "$*"
  read -r -p "  Press Enter when done. " _
}
ask() { local v; read -r -p "  $1: " v; printf '%s' "$v"; }
yes_no() {
  local v
  while :; do
    read -r -p "  $1 [y/n]: " v
    case "$v" in y | Y) return 0 ;; n | N) return 1 ;; esac
  done
}
pass() { printf 'PASS  %s\n' "$*" | tee -a "$EV/checks.txt"; }
fail() { printf 'FAIL  %s\n' "$*" | tee -a "$EV/checks.txt"; }
check() { # check "what" <command...>: pass when the command succeeds
  local what="$1"
  shift
  if "$@"; then pass "$what"; else fail "$what"; fi
}
save() { tee "$EV/$1.json" | jq .; }

op() { # op METHOD PATH: an operator call, answer on stdout
  curl -sS -X "$1" "$ORIGIN$2" -H "Authorization: Bearer $OPERATOR_TOKEN"
}

# One MCP tools/call on /mcp/streamable as the agent, the way the test's
# helper makes it. The decoded answer is saved as NAME.json and printed.
mcp() { # mcp NAME TOOL ARGS_JSON
  AGENT_CALLS=$((AGENT_CALLS + 1))
  curl -sS "$ORIGIN/mcp/streamable" \
    -H "Authorization: Bearer $AGENT_TOKEN" \
    -H 'Accept: application/json' -H 'Content-Type: application/json' \
    -d "$(jq -cn --arg name "$2" --argjson args "$3" \
      '{jsonrpc: "2.0", id: 1, method: "tools/call", params: {name: $name, arguments: $args}}')" |
    jq 'if .error then .error
        elif .result then (.result | if .isError then {refused: .content[0].text}
                                     else (.content[0].text | fromjson? // .) end)
        else . end' | save "$1"
}
field() { jq -r "$2 // empty" "$EV/$1.json"; }

entry_for() { # entry_for EMAIL: the newest waitlist entry for the address
  op GET /operator/waitlist |
    jq --arg e "$1" '[.[] | select(.email == $e)] | sort_by(.created_at) | last
                     | {id, status, workspace_id, created_at}'
}

sign_up() { # sign_up LETTER: steps 1a to 1d for person A or B; sets SIGNED_WS
  local who="$1" address entry invite
  bold "Step 1${who}: person ${who} joins, is invited and signs up"
  address="$(ask "Address ${who} (a fresh one that never signed in)")"
  browser "Profile ${who}: open $ORIGIN/, choose Join the waitlist, enter ${address}, use case \"launch gate run\", tick consent, send. Screenshot the thanks message."
  entry_for "$address" | save "waitlist-${who}-before"
  entry="$(field "waitlist-${who}-before" .id)"
  [ -n "$entry" ] || { fail "1${who}: no waitlist entry for ${address}"; exit 1; }
  check "1${who}: the entry is waiting" test "$(field "waitlist-${who}-before" .status)" = waiting
  invite="$(op POST "/operator/waitlist/$entry/invite")"
  jq '{invite_expires_at}' <<<"$invite" | save "invite-${who}"
  printf '\n  Give person %s this link (it is the invite; it is not saved):\n  %s%s\n' \
    "$who" "$ORIGIN" "$(jq -r .signup_path <<<"$invite")"
  invite=""
  browser "Profile ${who}: open that link, create workspace launch-gate-$(tr 'AB' 'ab' <<<"$who")-${STAMP}, sign in with ${address} and its one-time code. Screenshot the Board (only #general) and People (one member). Save $ORIGIN/auth/session from this profile."
  entry_for "$address" | save "waitlist-${who}-after"
  check "1${who}: the entry is signed_up" test "$(field "waitlist-${who}-after" .status)" = signed_up
  SIGNED_WS="$(field "waitlist-${who}-after" .workspace_id)"
}

bold "Launch gate run against $ORIGIN, evidence in $EV"
printf '  Operator token: '
read -rs OPERATOR_TOKEN
echo

bold "Step 0: the instance"
curl -sS "$ORIGIN/health" | save health
check "0: /health is ok" test "$(field health .status)" = ok
VERSION="$(field health .version)"
check "0: /health reports ${EXPECT_VERSION:-$VERSION}" test "$VERSION" = "${EXPECT_VERSION:-$VERSION}"
browser "Any browser: open $ORIGIN/status and screenshot it."

sign_up A
WS_A="$SIGNED_WS"
echo "WS_A=$WS_A"
sign_up B
WS_B="$SIGNED_WS"
echo "WS_B=$WS_B"
check "1: two sign-ups made two workspaces" test -n "$WS_A" -a -n "$WS_B" -a "$WS_A" != "$WS_B"

bold "Step 1f: person A invites a second person and revokes"
browser "Profile A: People, Invite a person, a third fresh address, Member, Invite. Then Revoke that row. Screenshot before and after (no link). Then in profile B open $ORIGIN/ui/api/workspaces/$WS_A/invites."
if yes_no "Did the row read revoked, and was profile B refused A's invites"; then
  pass "1f: A invited and revoked; B was refused A's invites"
else
  fail "1f: invite, revoke or B's refusal"
fi

bold "Step 2: a channel for the work"
browser "Profile A: Tools, Workspace, New channel \"launch\", Create."

bold "Step 3a: person A connects Claude Code over OAuth"
browser "In an empty scratch directory: claude mcp add --transport http maidan-gate-a $ORIGIN/mcp/streamable ; then claude, /mcp, authenticate in profile A. The consent page must name launch-gate-a-${STAMP}; screenshot it, Allow. Ask Claude to call whoami, then list_channels with $WS_A. Keep the count of tool calls Claude makes on maidan-gate-a."
if yes_no "Did consent name launch-gate-a-${STAMP} and whoami name workspace $WS_A"; then
  pass "3a: A's OAuth consent and whoami name workspace A"
else
  fail "3a: A's consent or whoami"
fi

bold "Step 3b: person A adds the agent"
browser "Profile A: People, Add an agent, name gate-agent, client Claude Code. Keep its worker token on the clipboard for the next prompt, then press I have it."
printf '  Agent token: '
read -rs AGENT_TOKEN
echo
mcp whoami whoami '{}'
AGENT="$(field whoami .member_id)"
check "3b: the agent's token works in workspace A" test "$(field whoami .workspace_id)" = "$WS_A"
check "3b: the agent holds thread:transition" jq -e '.capabilities | index("thread:transition")' "$EV/whoami.json" >/dev/null
mcp members list_members "$(jq -cn --arg w "$WS_A" '{workspace_id: $w}')"
PERSON_A="$(jq -r '[.. | objects | select(.kind? == "human") | .id] | first // empty' "$EV/members.json")"
check "3b: workspace A holds person A and gate-agent" test -n "$PERSON_A" -a -n "$AGENT"
mcp channels list_channels "$(jq -cn --arg w "$WS_A" '{workspace_id: $w}')"
CH="$(jq -r '[.. | objects | select(.name? == "launch") | .id] | first // empty' "$EV/channels.json")"
check "2: workspace A has general and launch" test -n "$CH" -a "$(jq '[.. | objects | select(.name? == "general")] | length' "$EV/channels.json")" -ge 1
echo "PERSON_A=$PERSON_A AGENT=$AGENT CH=$CH"

bold "Step 3c: person B connects Claude Code over OAuth"
browser "As 3a with server maidan-gate-b in profile B: consent must name launch-gate-b-${STAMP}; whoami must name workspace $WS_B. Keep the count of tool calls on maidan-gate-b."
if yes_no "Did consent name launch-gate-b-${STAMP} and whoami name workspace $WS_B"; then
  pass "3c: B's OAuth consent and whoami name workspace B"
else
  fail "3c: B's consent or whoami"
fi

bold "Step 4: the agent runs a change"
mcp create create_thread "$(jq -cn --arg c "$CH" '{channel_id: $c, title: "Launch gate run",
  attributes: {kind: "change", engine: "session", acceptance: "the launch gate passes"}}')"
T="$(jq -r '.id // .thread.id // empty' "$EV/create.json")"
check "4: create_thread returned a thread" test -n "$T"
mcp owner set_thread_owner "$(jq -cn --arg t "$T" --arg o "$PERSON_A" '{thread_id: $t, owner_id: $o}')"
mcp claim claim_next_workspace_thread "$(jq -cn --arg w "$WS_A" \
  '{workspace_id: $w, engine: "session", lease_secs: 1800}')"
L="$(jq -r '[.. | objects | .claim_lease_id? // empty] | first // empty' "$EV/claim.json")"
check "4: the claim took T" test "$(jq -r '[.. | objects | select(.id? == "'"$T"'")] | length' "$EV/claim.json")" -ge 1
check "4: the claim's assignee is the agent" test "$(jq -r '[.. | objects | .assignee_id? // empty] | first // empty' "$EV/claim.json")" = "$AGENT"
mcp ack acknowledge_claim "$(jq -cn --arg t "$T" --arg l "$L" '{thread_id: $t, claim_lease_id: $l}')"
mcp result set_thread_result "$(jq -cn --arg t "$T" --arg l "$L" \
  '{thread_id: $t, claim_lease_id: $l, result: {summary: "the launch path works end to end"}}')"
mcp usage report_usage "$(jq -cn --arg t "$T" --arg l "$L" --arg u "$(uuidgen | tr 'A-Z' 'a-z')" '{
  thread_id: $t, usage_report_id: $u, claim_lease_id: $l, model: "claude-opus-4-5",
  tokens: {input: 1200, output: 340, cache_read: 0, cache_write_5m: 0, cache_write_1h: 0},
  usd_micros: 8700,
  price_snapshot: {input_usd_micros_per_million: 3000000, output_usd_micros_per_million: 15000000,
    cache_read_usd_micros_per_million: 300000, cache_write_5m_usd_micros_per_million: 3750000,
    cache_write_1h_usd_micros_per_million: 6000000},
  turns: 1}')"
check "4: report_usage answered stopped: false" test "$(jq -r '.stopped' "$EV/usage.json")" = false
mcp review transition_thread "$(jq -cn --arg t "$T" '{thread_id: $t, action: "start_review"}')"
check "4: start_review left the thread in_review" test "$(jq -r '[.. | objects | .state? // empty] | first // empty' "$EV/review.json")" = in_review
curl -sS "$ORIGIN/threads/$T/usage-rollup" -H "Authorization: Bearer $AGENT_TOKEN" | save usage-rollup
browser "Profile A: the Board's launch channel shows the task in review. Screenshot it."

bold "Step 5: the agent cannot close its own work"
mcp close transition_thread "$(jq -cn --arg t "$T" '{thread_id: $t, action: "close"}')"
check "5: the agent's close is refused for separation of duties" \
  grep -q 'separation of duties' "$EV/close.json"
mcp release release_claim "$(jq -cn --arg t "$T" --arg l "$L" '{thread_id: $t, claim_lease_id: $l}')"
check "5: the release succeeded" test "$(jq -r 'if type == "object" then (has("refused") or has("code")) else false end' "$EV/release.json")" = false

bold "Step 8, during: the second tenant's client is refused"
browser "In Claude Code on maidan-gate-b only: get_thread_context with thread_id $T, then claim_next_workspace_thread with workspace_id $WS_A. Both must be refused with nothing of workspace A."
if yes_no "Were both refused, with nothing of workspace A in either answer"; then
  pass "8: B's client was refused A's thread and A's queue"
else
  fail "8: B's client saw something of A"
fi

bold "Step 6: the person reviews and closes"
browser "Profile A: open $ORIGIN/ui/api/threads/$T/review-packet and note evidence_root. Then Needs you (or the launch channel), open the task, Approve, then Close task from its menu. Screenshot the closed task sheet."
curl -sS "$ORIGIN/threads/$T" -H "Authorization: Bearer $AGENT_TOKEN" | save closed
check "6: the thread is closed" test "$(field closed .state)" = closed

bold "Step 8, after: the second person sees nothing"
browser "Profile B: open $ORIGIN/ui/api/threads/$T , $ORIGIN/ui/api/threads/$T/review-packet and $ORIGIN/ui/api/channels/$CH/threads ; then the Board and People. Save the three answers and two screenshots."
if yes_no "Did all three answer 403 or 404 with nothing of A, and B's Board and People show only B"; then
  pass "8: B's browser was refused everything of A"
else
  fail "8: B's browser saw something of A"
fi

bold "Step 7: metering"
CALLS_A="$(ask "Tool calls Claude made on maidan-gate-a (3a)")"
CALLS_B="$(ask "Tool calls Claude made on maidan-gate-b (3c and 8)")"
echo "  Waiting 75 seconds for the meters to flush (keep the console open)..."
sleep 75
op GET /operator/usage-meters |
  jq --arg a "$WS_A" --arg b "$WS_B" '[.[] | select(.workspace_id == $a or .workspace_id == $b)]' |
  save usage-meters
meter() { jq -r --arg w "$1" --arg m "$2" \
  '[.[] | select(.workspace_id == $w) | .meters[] | select(.meter == $m) | .quantity] | add // 0' \
  "$EV/usage-meters.json"; }
WANT_A=$((AGENT_CALLS + CALLS_A))
GOT_A="$(meter "$WS_A" mcp_calls)"
check "7: A's mcp_calls is ${WANT_A} (${AGENT_CALLS} agent + ${CALLS_A} Claude), got ${GOT_A}" test "$GOT_A" -eq "$WANT_A"
check "7: A's requests is at least its mcp_calls" test "$(meter "$WS_A" requests)" -ge "$GOT_A"
check "7: B's mcp_calls is ${CALLS_B}, got $(meter "$WS_B" mcp_calls)" test "$(meter "$WS_B" mcp_calls)" -eq "$CALLS_B"

bold "Cleanup"
browser "In profiles A and B: $ORIGIN/ui/oauth/grants, revoke the Claude Code grant. Then claude mcp remove maidan-gate-a and claude mcp remove maidan-gate-b."
unset AGENT_TOKEN
for who in A B; do
  ws="$WS_A"
  [ "$who" = B ] && ws="$WS_B"
  browser "Profile ${who}: Tools, Connect, Mint an admin token; keep it for the next prompt."
  printf '  Admin token for workspace %s: ' "$ws"
  read -rs ADMIN_TOKEN
  echo
  confirm="$(ask "Type the workspace id ${ws} to erase it permanently")"
  if [ "$confirm" = "$ws" ]; then
    curl -sS -X DELETE "$ORIGIN/workspaces/$ws" \
      -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
      -d "$(jq -cn --arg w "$ws" '{confirm_workspace_id: $w}')" | save "erase-${who}"
    pass "cleanup: erase of ${who} sent"
  else
    fail "cleanup: ${who} not erased (the id did not match)"
  fi
  unset ADMIN_TOKEN
done
unset OPERATOR_TOKEN
browser "Remove both test users from Cloudflare Zero Trust, then close both browser profiles."

{
  echo "# Launch gate run: $ORIGIN, ${VERSION}, $(date -u +%Y-%m-%dT%H:%MZ)"
  echo
  echo "WS_A=$WS_A WS_B=$WS_B PERSON_A=$PERSON_A AGENT=$AGENT CH=$CH T=$T"
  echo
  echo '```'
  cat "$EV/checks.txt"
  echo '```'
  echo
  echo "Screenshots to add: /status; A's and B's thanks, Board and People; People before and after the revoke;"
  echo "both consent pages; the launch channel in review; the closed task sheet; B's Board and People."
} >"$EV/record.md"
bold "Done. $(grep -c '^PASS' "$EV/checks.txt") passed, $(grep -c '^FAIL' "$EV/checks.txt") failed. Record: $EV/record.md"
