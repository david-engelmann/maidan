#!/usr/bin/env bash
# Multi-replica scale-out smoke.
#
# Brings up two maidan-server replicas behind an nginx round-robin LB, sharing
# one Postgres and one MinIO object store, then drives REST cross-replica paths
# through the LB (so requests land on either replica with no session affinity):
#   * a workspace/channel/thread round-trip — write on one replica,
#     read back through the LB (typically the other replica);
#   * an embedding reindex job started on one
#     replica and polled to completion on another.
# Every call carries a bearer: maidan-r1 seeds the shared database's dev admin
# (MAIDAN_DEV_ADMIN_TOKEN in compose.yaml), which opens the smoke's workspace,
# and that workspace's own admin token does the rest.
#
# Coverage split: the WebSocket/SSE cross-replica paths (resource notifications,
# presence/typing) and the app-OAuth mint-then-exchange path need real auth and
# a live stream, so they are proven by the in-process Rust two-replica e2es
# (two_replica_resource_notification_e2e, two_replica_stateless_subscription_e2e,
# two_replica_presence_e2e, two_replica_durable_state_e2e), not re-run here. This script proves the
# container topology (shared Postgres + object store + LB) holds the REST paths.
#
# Usage: scripts/scale-out-smoke.sh   (run from repo root; needs docker, curl, jq)
set -euo pipefail

LB_PORT="${MAIDAN_SCALE_LB_PORT:-8090}"
LB="http://localhost:${LB_PORT}"
COMPOSE=(docker compose -f compose.yaml --profile scale)
DEV_ADMIN_TOKEN="${MAIDAN_DEV_ADMIN_TOKEN:-maidan-dev-admin-token-local-stacks-only}"

cleanup() {
  echo "=== tearing down ==="
  "${COMPOSE[@]}" down -v || true
}
trap cleanup EXIT

fail() {
  echo "::error::$*"
  echo "=== maidan-r1 ===" && "${COMPOSE[@]}" logs maidan-r1 2>&1 | tail -50 || true
  echo "=== maidan-r2 ===" && "${COMPOSE[@]}" logs maidan-r2 2>&1 | tail -50 || true
  echo "=== lb ===" && "${COMPOSE[@]}" logs lb 2>&1 | tail -20 || true
  exit 1
}

echo "=== building images ==="
# CI loads maidan-server:dev and maidan-postgres:dev from the smoke image job,
# which builds them once for every smoke.
if [ "${MAIDAN_SMOKE_PREBUILT:-}" != "1" ]; then
  docker build -t maidan-server:dev -f crates/maidan-server/Dockerfile .
  docker build -t maidan-postgres:dev -f docker/Dockerfile.db .
fi

echo "=== bringing up two replicas + LB ==="
"${COMPOSE[@]}" up -d

echo "=== waiting for LB → replica readiness ==="
ready=0
for _ in $(seq 1 90); do
  if curl -sf "$LB/health" >/tmp/scale_health.json 2>/dev/null \
     && jq -e '.status == "ok"' /tmp/scale_health.json >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
[ "$ready" = 1 ] || fail "LB health check timed out"
echo "LB healthy: $(cat /tmp/scale_health.json)"

# --- workspace + thread round-trip through the LB ---
echo "=== workspace/thread round-trip ==="
OPENED=$(curl -sf -X POST "$LB/operator/workspaces" -H 'content-type: application/json' \
  -H "Authorization: Bearer $DEV_ADMIN_TOKEN" \
  -d '{"name":"scale-smoke","admin_handle":"admin"}') || fail "open workspace"
WID=$(echo "$OPENED" | jq -r '.workspace.id')
TOKEN=$(echo "$OPENED" | jq -r '.token.secret')
AUTH=(-H "Authorization: Bearer $TOKEN")
CID=$(curl -sf -X POST "$LB/workspaces/$WID/channels" -H 'content-type: application/json' \
  "${AUTH[@]}" -d '{"name":"general"}' | jq -r '.id') || fail "create channel"
TID=$(curl -sf -X POST "$LB/channels/$CID/threads" -H 'content-type: application/json' \
  "${AUTH[@]}" -d '{}' | jq -r '.id') || fail "create thread"
# Read it back (likely a different replica) — shared store must serve it.
curl -sf "${AUTH[@]}" "$LB/threads/$TID" | jq -e --arg id "$TID" '.id == $id' >/dev/null \
  || fail "thread not readable cross-replica"
echo "thread round-trip OK ($TID)"

# --- reindex job: start, then poll status (any replica serves it from the store) ---
echo "=== reindex job start → poll ==="
JOB=$(curl -sf -X POST "$LB/operator/reindex-embeddings" -H 'content-type: application/json' \
  "${AUTH[@]}" -d "{\"workspace_id\":\"$WID\"}" | jq -r '.job_id') || fail "start reindex"
done_job=0
for _ in $(seq 1 60); do
  st=$(curl -sf "${AUTH[@]}" "$LB/operator/reindex-embeddings/$JOB" | jq -r '.status') || true
  if [ "$st" = "completed" ]; then done_job=1; break; fi
  [ "$st" = "failed" ] && fail "reindex job failed"
  sleep 1
done
[ "$done_job" = 1 ] || fail "reindex job did not complete (last status: ${st:-none})"
echo "reindex cross-replica status OK"

echo "=== scale-out smoke PASSED ==="
