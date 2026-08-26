#!/usr/bin/env bash
# Two-agent demo for the Maidan quickstart (Cluster 278). Creates a workspace, two
# agent members, a channel and a thread, then has one agent post and the other read
# the shared thread and reply, proving the messages are durable shared state.
#
# Assumes the quickstart stack is up (docker compose -f compose.quickstart.yaml up -d)
# with auth disabled. Override the target with MAIDAN_URL.
set -euo pipefail

BASE_URL="${MAIDAN_URL:-http://127.0.0.1:8080}"

for cmd in curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

post() {
  # post <path> <json-body>  -> response body
  curl -fsS -H 'content-type: application/json' -X POST "${BASE_URL}$1" --data "$2"
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

workspace=$(post /workspaces '{"name":"two-agent-demo"}' | jq -er '.id')
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
curl -fsS "${BASE_URL}/threads/${thread}/context" | jq '.messages[] | {author_id, body}'

# Agent B (reviewer) reads the thread above, then replies into the same durable thread.
post "/threads/${thread}/messages" \
  "$(jq -cn --arg a "$reviewer" '{author_id:$a, body:"Highest risk is first-run onboarding: the quickstart must work from a clean machine."}')" \
  >/dev/null

echo
echo "planner reads the reply (both messages are durable shared state):"
curl -fsS "${BASE_URL}/threads/${thread}/context" | jq '.messages[] | {author_id, body}'

echo
echo "demo resources:"
printf '  workspace: %s\n  planner:   %s\n  reviewer:  %s\n  channel:   %s\n  thread:    %s\n' \
  "$workspace" "$planner" "$reviewer" "$channel" "$thread"
