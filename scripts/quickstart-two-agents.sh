#!/usr/bin/env bash
# Two-agent demo for the Maidan quickstart (Cluster 278; token-based since 313).
# Creates two agent members, a channel and a thread, then has one agent post and the
# other read the shared thread and reply, proving the messages are durable shared state.
#
# Default (secure) path — the quickstart stack runs with auth ON, so mint a token first:
#   docker compose -f compose.quickstart.yaml up -d --build
#   docker compose -f compose.quickstart.yaml exec maidan maidan init --workspace demo
#   MAIDAN_TOKEN=<token> MAIDAN_WORKSPACE=<workspace-id> ./scripts/quickstart-two-agents.sh
#
# `maidan init` prints both the admin bearer token and the workspace id; content
# operations (channel/thread/message) are authenticated with the token.
#
# Local-only path — if you layered compose.quickstart.insecure.yaml (auth disabled),
# run with no MAIDAN_TOKEN and the script seeds its own workspace unauthenticated.
#
# Override the target URL with MAIDAN_URL.
set -euo pipefail

BASE_URL="${MAIDAN_URL:-http://127.0.0.1:8080}"
TOKEN="${MAIDAN_TOKEN:-}"
WORKSPACE_IN="${MAIDAN_WORKSPACE:-}"

for cmd in curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

# Auth header is present only in token mode; empty array in insecure mode.
AUTH=()
if [ -n "$TOKEN" ]; then
  AUTH=(-H "authorization: Bearer ${TOKEN}")
  if [ -z "$WORKSPACE_IN" ]; then
    echo "MAIDAN_TOKEN is set but MAIDAN_WORKSPACE is not." >&2
    echo "Run 'maidan init --workspace demo' and pass the printed workspace id as" >&2
    echo "MAIDAN_WORKSPACE (the admin token already owns that workspace)." >&2
    exit 1
  fi
  echo "auth: bearer token (default-secure path)"
else
  echo "auth: DISABLED (local-only path — layer compose.quickstart.insecure.yaml)"
fi

post() {
  # post <path> <json-body>  -> response body. The ${AUTH[@]+…} guard keeps an empty
  # auth array from tripping `set -u` on bash 3.2 (macOS) in the insecure path.
  curl -fsS -H 'content-type: application/json' ${AUTH[@]+"${AUTH[@]}"} -X POST "${BASE_URL}$1" --data "$2"
}
get() {
  curl -fsS ${AUTH[@]+"${AUTH[@]}"} "${BASE_URL}$1"
}

echo "waiting for Maidan at ${BASE_URL} ..."
for _ in $(seq 1 60); do
  if curl -fsS "${BASE_URL}/health" >/dev/null 2>&1; then ready=1; break; fi
  sleep 1
done
if [ "${ready:-0}" != "1" ]; then
  echo "Maidan did not become healthy. Check: docker compose -f compose.quickstart.yaml logs" >&2
  exit 1
fi

# In secure mode `maidan init` already created the workspace; reuse it. In insecure mode
# there is no token yet, so seed a fresh workspace over the bootstrap route.
if [ -n "$TOKEN" ]; then
  workspace="$WORKSPACE_IN"
else
  workspace=$(post /workspaces '{"name":"two-agent-demo"}' | jq -er '.id')
fi

# The two agent members are seeded over the bootstrap route (open in both quickstart
# modes: auth-disabled, or MAIDAN_BOOTSTRAP=1 with auth on).
planner=$(post "/workspaces/${workspace}/members" '{"handle":"planner","kind":"agent"}' | jq -er '.id')
reviewer=$(post "/workspaces/${workspace}/members" '{"handle":"reviewer","kind":"agent"}' | jq -er '.id')
channel=$(post "/workspaces/${workspace}/channels" '{"name":"coordination"}' | jq -er '.id')
thread=$(post "/channels/${channel}/threads" '{"title":"launch-plan"}' | jq -er '.id')

# Agent A (planner) posts a request into the shared thread.
post "/threads/${thread}/messages" \
  "$(jq -cn --arg a "$planner" '{author_id:$a, body:"Reviewer: what is the highest-risk item on the launch checklist?"}')" \
  >/dev/null

echo
echo "reviewer reads the shared thread:"
get "/threads/${thread}/context" | jq '.messages[] | {author_id, body}'

# Agent B (reviewer) reads the thread above, then replies into the same durable thread.
post "/threads/${thread}/messages" \
  "$(jq -cn --arg a "$reviewer" '{author_id:$a, body:"Highest risk is first-run onboarding: the quickstart must work from a clean machine."}')" \
  >/dev/null

echo
echo "planner reads the reply (both messages are durable shared state):"
get "/threads/${thread}/context" | jq '.messages[] | {author_id, body}'

echo
echo "demo resources:"
printf '  workspace: %s\n  planner:   %s\n  reviewer:  %s\n  channel:   %s\n  thread:    %s\n' \
  "$workspace" "$planner" "$reviewer" "$channel" "$thread"
