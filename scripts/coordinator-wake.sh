#!/usr/bin/env bash
# Wait, at no token cost, until something needs a coordinator, then print one
# digest of everything pending and exit. A Claude Code session runs it as a
# background command: the harness's completion notice is what wakes the
# session, so an idle swarm costs the coordinator nothing.
#
#   MAIDAN_URL=https://maidan.example MAIDAN_TOKEN=... scripts/coordinator-wake.sh
#
# It long-polls wait_for_notification on $MAIDAN_URL/mcp/coordinator with the
# longest timeout the server allows. A notification of an attention kind (a
# question, a stalled claim, a handoff, a failed check, a block, a failed
# delivery, an intake proposal, a refused spawn, a change request, an approval
# gate, a mention) ends the wait: it lists every unread notification of those
# kinds and prints one line each, oldest first, then a total. Other kinds are
# skipped and the wait goes on. It never marks anything read: the coordinator
# does that after acting, so what it has not handled wakes it again at once.
#
# Exit status: 0 with a digest; 2 with "nothing pending" when --max-wait
# lapsed; 1 when the token is refused or the server rejects the call.
#
# Options:
#   --max-wait SECONDS  give up after this long (default: wait forever)
#   --kinds a,b,c       the attention kinds (default: DEFAULT_KINDS below)
#   --member UUID       the coordinator's member id (default: from whoami)
#   --dry-run           print the request bodies, without the token, and exit
set -euo pipefail

DEFAULT_KINDS="mention_recorded,status_declared,claim_stalled,claim_unacknowledged,claim_expired,claim_failed,thread_handoff,thread_notice,thread_blocked,result_delivery_failed,intake_proposed,thread_spawn_denied,review_submitted,approval_requested,wait_timed_out"
# The server's ceiling for one wait (MAX_WAIT_MS in the MCP member tools).
MAX_WAIT_MS=300000
# Unread rows one digest reads; the server's own cap for list_notifications.
DIGEST_LIMIT=500
BACKOFF_MAX=60

max_wait=""
kinds="$DEFAULT_KINDS"
member=""
dry_run=0

usage() { sed -n '2,/^set -euo/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'; }
die() {
  printf 'coordinator-wake: %s\n' "$1" >&2
  exit "${2:-1}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --max-wait)
      [ $# -ge 2 ] || die "--max-wait needs a number of seconds"
      max_wait="$2"
      shift 2
      ;;
    --kinds)
      [ $# -ge 2 ] || die "--kinds needs a comma-separated list"
      kinds="$2"
      shift 2
      ;;
    --member)
      [ $# -ge 2 ] || die "--member needs a member id"
      member="$2"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

if [ -n "$max_wait" ] && ! [[ "$max_wait" =~ ^[0-9]+$ ]]; then
  die "--max-wait takes whole seconds, not '$max_wait'"
fi
command -v jq >/dev/null || die "needs jq"
kinds_json="$(jq -cn --arg k "$kinds" '$k | split(",") | map(select(length > 0))')"
[ "$(jq 'length' <<<"$kinds_json")" -gt 0 ] || die "--kinds is empty"

# One JSON-RPC tools/call body, as docs/Launch Gate.md's mcp() helper sends it.
body() {
  jq -cn --arg name "$1" --argjson args "$2" \
    '{jsonrpc: "2.0", id: 1, method: "tools/call", params: {name: $name, arguments: $args}}'
}

wait_args() {
  jq -cn --arg m "$1" --argjson t "$2" --argjson s "$3" \
    '{member_id: $m, timeout_ms: $t, since_log_id: $s}'
}

list_args() {
  jq -cn --arg m "$1" --argjson l "$2" --argjson u "$3" \
    '{member_id: $m, unread_only: $u, limit: $l}'
}

if [ "$dry_run" = 1 ]; then
  shown_member="${member:-MEMBER_ID_FROM_WHOAMI}"
  printf 'POST %s/mcp/coordinator\n' "${MAIDAN_URL:-\$MAIDAN_URL}"
  [ -n "$member" ] || body whoami '{}'
  body list_notifications "$(list_args "$shown_member" "$DIGEST_LIMIT" true)"
  body list_notifications "$(list_args "$shown_member" 1 false)"
  body wait_for_notification "$(wait_args "$shown_member" "$MAX_WAIT_MS" 0)"
  printf 'attention kinds: %s\n' "$(jq -r 'join(",")' <<<"$kinds_json")"
  exit 0
fi

command -v curl >/dev/null || die "needs curl"
[ -n "${MAIDAN_URL:-}" ] || die "set MAIDAN_URL to the Maidan origin"
[ -n "${MAIDAN_TOKEN:-}" ] || die "set MAIDAN_TOKEN to the coordinator's token"
endpoint="${MAIDAN_URL%/}/mcp/coordinator"
started=$(date +%s)

# Seconds left under --max-wait; empty when there is no limit.
remaining() {
  [ -n "$max_wait" ] || return 0
  echo $((max_wait - ($(date +%s) - started)))
}

# Said on the script's own stdout even from inside a command substitution,
# where plain stdout would be captured instead of shown.
exec 3>&1
nothing_pending() {
  echo "nothing pending" >&3
  exit 2
}

# Call one tool and print its decoded result. Exits 1 on a refused token or a
# call the server rejects; returns 75 on a failure worth retrying. The token
# reaches curl through a file descriptor, so it is never in argv or output.
call() {
  local name="$1" args="$2" max_time="$3" out status code
  out="$(body "$name" "$args" | curl -sS --max-time "$max_time" -w '\n%{http_code}' \
    -H @<(printf 'Authorization: Bearer %s\n' "$MAIDAN_TOKEN") \
    -H 'Accept: application/json' -H 'Content-Type: application/json' \
    --data-binary @- "$endpoint" 2>/dev/null)" || {
    printf 'coordinator-wake: %s: no answer from %s; retrying\n' "$name" "$endpoint" >&2
    return 75
  }
  status="${out##*$'\n'}"
  out="${out%$'\n'*}"
  case "$status" in
    401 | 403) die "$name: the token was refused (HTTP $status); check MAIDAN_TOKEN" ;;
    408 | 425 | 429 | 5??)
      printf 'coordinator-wake: %s: HTTP %s; retrying\n' "$name" "$status" >&2
      return 75
      ;;
    2??) ;;
    *) die "$name: HTTP $status from $endpoint" ;;
  esac
  code="$(jq -r '.error.code // empty' <<<"$out" 2>/dev/null)" ||
    die "$name: the answer is not JSON-RPC"
  case "$code" in
    "") ;;
    -32001 | -32003) die "$name: refused: $(jq -r '.error.message' <<<"$out")" ;;
    -32603)
      printf 'coordinator-wake: %s: server error; retrying\n' "$name" >&2
      return 75
      ;;
    *) die "$name: error $code: $(jq -r '.error.message' <<<"$out")" ;;
  esac
  if [ "$(jq -r '.result.isError // false' <<<"$out")" = true ]; then
    die "$name: refused: $(jq -r '.result.content[0].text' <<<"$out")"
  fi
  # A null result (a wait that lapsed) must stay null, which `fromjson? // .`
  # would turn back into the text.
  jq -c '.result.content[0].text as $t | try ($t | fromjson) catch $t' <<<"$out"
}

# Call, retrying a transient failure with capped exponential backoff while
# --max-wait allows.
call_retrying() {
  local delay=1 left out rc
  while :; do
    rc=0
    out="$(call "$@")" || rc=$?
    [ "$rc" = 0 ] && { printf '%s\n' "$out"; return 0; }
    [ "$rc" = 75 ] || exit "$rc"
    left="$(remaining)"
    if [ -n "$left" ] && [ "$left" -le "$delay" ]; then
      nothing_pending
    fi
    sleep "$delay"
    delay=$((delay * 2 > BACKOFF_MAX ? BACKOFF_MAX : delay * 2))
  done
}

# Every unread notification of an attention kind, one line each, oldest
# first, then a total.
digest() {
  local unread
  unread="$(call_retrying list_notifications "$(list_args "$member" "$DIGEST_LIMIT" true)" 30)"
  jq -r --argjson kinds "$kinds_json" '
    def age: (now - (.created_at | sub("\\.[0-9]+"; "") | sub("\\+00:00$"; "Z") | fromdateiso8601))
      | floor
      | if . < 60 then "\(.)s" elif . < 3600 then "\(. / 60 | floor)m"
        elif . < 86400 then "\(. / 3600 | floor)h" else "\(. / 86400 | floor)d" end;
    [.[] | select(.kind as $k | $kinds | any(. == $k))] | sort_by(.source_log_id)
    | (.[] | "\(age) ago  \(.kind)  thread=\(.thread_id // "-")  \"\(.title // "")\"  \(.reason // "")  [notification \(.id)]"),
      "\(length) pending: \(group_by(.kind) | map("\(length) \(.[0].kind)") | join(", "))"
  ' <<<"$unread"
}

pending_count() {
  jq --argjson kinds "$kinds_json" '[.[] | select(.kind as $k | $kinds | any(. == $k))] | length'
}

if [ -z "$member" ]; then
  member="$(call_retrying whoami '{}' 30 | jq -r '.member_id // empty')"
  [ -n "$member" ] || die "whoami named no member; pass --member"
fi

# Something already waiting is a reason to wake now, not after the next event.
unread="$(call_retrying list_notifications "$(list_args "$member" "$DIGEST_LIMIT" true)" 30)"
if [ "$(pending_count <<<"$unread")" -gt 0 ]; then
  digest
  exit 0
fi
# Wait for notifications newer than the newest one there is, read or not, so
# an old unread row of another kind does not end each wait at once.
cursor="$(call_retrying list_notifications "$(list_args "$member" 1 false)" 30 |
  jq '[.[].source_log_id] | max // 0')"

while :; do
  timeout_ms=$MAX_WAIT_MS
  left="$(remaining)"
  if [ -n "$left" ]; then
    [ "$left" -gt 0 ] || nothing_pending
    [ $((left * 1000)) -lt "$timeout_ms" ] && timeout_ms=$((left * 1000))
  fi
  got="$(call_retrying wait_for_notification "$(wait_args "$member" "$timeout_ms" "$cursor")" \
    $((timeout_ms / 1000 + 30)))"
  [ "$got" = null ] && continue
  cursor="$(jq --argjson c "$cursor" '[.source_log_id, $c] | max' <<<"$got")"
  if [ "$(jq --argjson kinds "$kinds_json" '.kind as $k | $kinds | any(. == $k)' <<<"$got")" = true ]; then
    digest
    exit 0
  fi
done
