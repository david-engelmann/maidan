#!/usr/bin/env bash
# Pull-path federation smoke: instance A emits an event; instance B's poll worker
# ingests it via the registered peer (remote_workspace_id points at A's workspace).
set -euo pipefail

MAIDAN_A_URL="${MAIDAN_A_URL:-http://localhost:8080}"
MAIDAN_B_URL="${MAIDAN_B_URL:-http://localhost:8081}"
# URL stored on the peer for the poll worker (container network); defaults to MAIDAN_A_URL for local curl-only runs.
MAIDAN_A_PEER_URL="${MAIDAN_A_PEER_URL:-${MAIDAN_A_URL}}"
POLL_WAIT_SECS="${FEDERATION_PULL_WAIT_SECS:-45}"

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

echo "creating message on A…"
ws_a=$(json_post "${MAIDAN_A_URL}/workspaces" -d '{"name":"fed-pull-a"}' | jq -r '.id')
alice=$(json_post "${MAIDAN_A_URL}/workspaces/${ws_a}/members" \
  -d '{"handle":"alice","display_name":"Alice","kind":"human"}' | jq -r '.id')
channel=$(json_post "${MAIDAN_A_URL}/workspaces/${ws_a}/channels" \
  -d '{"name":"general"}' | jq -r '.id')
thread=$(json_post "${MAIDAN_A_URL}/channels/${channel}/threads" \
  -d '{"title":"federation-pull"}' | jq -r '.id')
json_post "${MAIDAN_A_URL}/threads/${thread}/messages" \
  -d "{\"author_id\":\"${alice}\",\"body\":\"hello pull federation\"}" >/dev/null

stored=$(curl -sf "${MAIDAN_A_URL}/workspaces/${ws_a}/events?after_id=0&limit=50" \
  | jq '[.[] | select(.kind == "message_posted")][0]')
if [[ -z "$stored" || "$stored" == "null" ]]; then
  echo "::error::no message_posted event on A" >&2
  exit 1
fi
remote_id=$(echo "$stored" | jq '.id')

echo "registering peer on B (local workspace + remote A workspace)…"
ws_b=$(json_post "${MAIDAN_B_URL}/workspaces" -d '{"name":"fed-pull-b"}' | jq -r '.id')
peer_body=$(json_post "${MAIDAN_B_URL}/workspaces/${ws_b}/peers" \
  -d "{\"name\":\"upstream-a\",\"base_url\":\"${MAIDAN_A_PEER_URL}\",\"remote_workspace_id\":\"${ws_a}\"}")
peer_secret=$(echo "$peer_body" | jq -r '.secret')

echo "waiting up to ${POLL_WAIT_SECS}s for B poll worker to ingest remote event ${remote_id}…"
deadline=$((SECONDS + POLL_WAIT_SECS))
found=0
while (( SECONDS < deadline )); do
  tail=$(curl -sf "${MAIDAN_B_URL}/workspaces/${ws_b}/events?after_id=0&limit=100")
  count=$(echo "$tail" | jq '[.[] | select(.kind == "message_posted")] | length')
  if [[ "$count" -ge 1 ]]; then
    found=1
    break
  fi
  sleep 2
done

if [[ "$found" != "1" ]]; then
  echo "::error::B did not ingest message_posted from A within ${POLL_WAIT_SECS}s" >&2
  echo "peer_secret prefix: ${peer_secret:0:8}…" >&2
  exit 1
fi

echo "federation pull compose smoke ok (remote_event_id=${remote_id})"
