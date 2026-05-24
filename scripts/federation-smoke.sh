#!/usr/bin/env bash
# Push-path federation smoke: instance A emits a message event; instance B
# ingests it via POST /a2a/v1/events and exposes it on the peer event tail.
set -euo pipefail

MAIDAN_A_URL="${MAIDAN_A_URL:-http://localhost:8080}"
MAIDAN_B_URL="${MAIDAN_B_URL:-http://localhost:8081}"

wait_health() {
  local base=$1
  local name=$2
  for _ in $(seq 1 60); do
    if curl -sf "${base}/health" >/dev/null; then
      return 0
    fi
    sleep 2
  done
  echo "::error::${name} health timed out (${base})" >&2
  return 1
}

json_post() {
  local url=$1
  shift
  curl -sf -X POST "$url" -H 'Content-Type: application/json' "$@"
}

echo "waiting for ${MAIDAN_A_URL} and ${MAIDAN_B_URL}…"
wait_health "$MAIDAN_A_URL" "maidan-a"
wait_health "$MAIDAN_B_URL" "maidan-b"

echo "registering peer on B…"
ws_b=$(json_post "${MAIDAN_B_URL}/workspaces" -d '{"name":"fed-smoke-b"}' | jq -r '.id')
peer_body=$(json_post "${MAIDAN_B_URL}/workspaces/${ws_b}/peers" \
  -d "{\"name\":\"upstream-a\",\"base_url\":\"${MAIDAN_A_URL}\"}")
peer_id=$(echo "$peer_body" | jq -r '.peer.id')
peer_secret=$(echo "$peer_body" | jq -r '.secret')

echo "creating message on A…"
ws_a=$(json_post "${MAIDAN_A_URL}/workspaces" -d '{"name":"fed-smoke-a"}' | jq -r '.id')
alice=$(json_post "${MAIDAN_A_URL}/workspaces/${ws_a}/members" \
  -d '{"handle":"alice","display_name":"Alice","kind":"human"}' | jq -r '.id')
channel=$(json_post "${MAIDAN_A_URL}/workspaces/${ws_a}/channels" \
  -d '{"name":"general"}' | jq -r '.id')
thread=$(json_post "${MAIDAN_A_URL}/channels/${channel}/threads" \
  -d '{"title":"federation"}' | jq -r '.id')
json_post "${MAIDAN_A_URL}/threads/${thread}/messages" \
  -d "{\"author_id\":\"${alice}\",\"body\":\"hello federation\"}" >/dev/null

stored=$(curl -sf "${MAIDAN_A_URL}/workspaces/${ws_a}/events?after_id=0&limit=50" \
  | jq '[.[] | select(.kind == "message_posted")][0]')
if [[ -z "$stored" || "$stored" == "null" ]]; then
  echo "::error::no message_posted event on A" >&2
  exit 1
fi
remote_id=$(echo "$stored" | jq '.id')

batch=$(jq -n \
  --arg peer_id "$peer_id" \
  --argjson remote_id "$remote_id" \
  --argjson event "$stored" \
  '{events: [{origin_peer_id: $peer_id, remote_event_id: $remote_id, event: $event}]}')

echo "pushing event batch to B…"
summary=$(curl -sf -X POST "${MAIDAN_B_URL}/a2a/v1/events" \
  -H "Authorization: Bearer ${peer_secret}" \
  -H 'Content-Type: application/json' \
  -d "$batch")
ingested=$(echo "$summary" | jq -r '.ingested')
if [[ "$ingested" != "1" ]]; then
  echo "::error::expected ingested=1, got ${summary}" >&2
  exit 1
fi

echo "reading peer event tail on B…"
tail=$(curl -sf "${MAIDAN_B_URL}/workspaces/${ws_b}/events?after_id=0&limit=50" \
  -H "Authorization: Bearer ${peer_secret}")
count=$(echo "$tail" | jq '[.[] | select(.kind == "message_posted")] | length')
if [[ "$count" -lt 1 ]]; then
  echo "::error::peer tail missing message_posted on B" >&2
  exit 1
fi

echo "federation compose smoke ok (ingested=${ingested}, tail message_posted=${count})"
