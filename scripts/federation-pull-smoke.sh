#!/usr/bin/env bash
# Pull-path federation smoke: instance A emits an event; instance B's poll worker
# ingests it via the registered peer (remote_workspace_id points at A's workspace).
set -euo pipefail

MAIDAN_A_URL="${MAIDAN_A_URL:-http://localhost:8080}"
MAIDAN_B_URL="${MAIDAN_B_URL:-http://localhost:8081}"
# URL stored on the peer for the poll worker (container network); defaults to MAIDAN_A_URL for local curl-only runs.
MAIDAN_A_PEER_URL="${MAIDAN_A_PEER_URL:-${MAIDAN_A_URL}}"
POLL_WAIT_SECS="${FEDERATION_PULL_WAIT_SECS:-45}"
# The dev admin compose.yaml seeds into the database A and B share. It only
# opens workspaces: each one's own admin token does the work inside it, since a
# token acts in its own workspace and nowhere else. B's poll worker reads A
# with the peer secret, which A resolves from the same shared database.
DEV_ADMIN_TOKEN="${MAIDAN_DEV_ADMIN_TOKEN:-maidan-dev-admin-token-local-stacks-only}"

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

# Open a workspace on the instance at $1 named $2; print its body, whose
# .workspace.id and .token.secret the caller reads.
open_workspace() {
  json_post "$1/operator/workspaces" \
    -H "Authorization: Bearer ${DEV_ADMIN_TOKEN}" \
    -d "{\"name\":\"$2\",\"admin_handle\":\"admin\"}"
}

echo "waiting for ${MAIDAN_A_URL} and ${MAIDAN_B_URL}…"
wait_health "$MAIDAN_A_URL" "maidan-a"
wait_health "$MAIDAN_B_URL" "maidan-b"

echo "creating thread on A…"
opened_a=$(open_workspace "$MAIDAN_A_URL" fed-pull-a)
ws_a=$(echo "$opened_a" | jq -r '.workspace.id')
token_a=$(echo "$opened_a" | jq -r '.token.secret')
channel=$(json_post "${MAIDAN_A_URL}/workspaces/${ws_a}/channels" \
  -H "Authorization: Bearer ${token_a}" \
  -d '{"name":"general"}' | jq -r '.id')
thread=$(json_post "${MAIDAN_A_URL}/channels/${channel}/threads" \
  -H "Authorization: Bearer ${token_a}" \
  -d '{"title":"federation-pull"}' | jq -r '.id')

stored=$(curl -sf "${MAIDAN_A_URL}/workspaces/${ws_a}/events?after_id=0&limit=50" \
  -H "Authorization: Bearer ${token_a}" \
  | jq '[.[] | select(.kind == "thread_created")][0]')
if [[ -z "$stored" || "$stored" == "null" ]]; then
  echo "::error::no thread_created event on A" >&2
  exit 1
fi
remote_id=$(echo "$stored" | jq '.id')

echo "registering peer on B (local workspace + remote A workspace)…"
opened_b=$(open_workspace "$MAIDAN_B_URL" fed-pull-b)
ws_b=$(echo "$opened_b" | jq -r '.workspace.id')
token_b=$(echo "$opened_b" | jq -r '.token.secret')
peer_body=$(json_post "${MAIDAN_B_URL}/workspaces/${ws_b}/peers" \
  -H "Authorization: Bearer ${token_b}" \
  -d "{\"name\":\"upstream-a\",\"base_url\":\"${MAIDAN_A_PEER_URL}\",\"remote_workspace_id\":\"${ws_a}\"}")
peer_secret=$(echo "$peer_body" | jq -r '.secret')

echo "waiting up to ${POLL_WAIT_SECS}s for B poll worker to ingest remote event ${remote_id}…"
deadline=$((SECONDS + POLL_WAIT_SECS))
found=0
while (( SECONDS < deadline )); do
  tail=$(curl -sf "${MAIDAN_B_URL}/workspaces/${ws_b}/events?after_id=0&limit=100" \
    -H "Authorization: Bearer ${token_b}")
  count=$(echo "$tail" | jq '[.[] | select(.kind == "thread_created")] | length')
  if [[ "$count" -ge 1 ]]; then
    found=1
    break
  fi
  sleep 2
done

if [[ "$found" != "1" ]]; then
  echo "::error::B did not ingest thread_created from A within ${POLL_WAIT_SECS}s" >&2
  echo "peer_secret prefix: ${peer_secret:0:8}…" >&2
  exit 1
fi

echo "federation pull compose smoke ok (remote_event_id=${remote_id})"
