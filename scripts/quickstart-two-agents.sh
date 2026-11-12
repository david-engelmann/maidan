#!/usr/bin/env bash
# Two-agent demo for the Maidan quickstart.
# Creates two agent members, a channel and a thread, then has one agent post and the
# other read the shared thread and reply, proving the messages are durable shared state.
#
# The quickstart stack authenticates every request, so mint a token first:
#   docker compose -f compose.quickstart.yaml up -d --build --wait
#   docker compose -f compose.quickstart.yaml exec maidan maidan init --workspace demo
#   MAIDAN_TOKEN=<token> MAIDAN_WORKSPACE=<workspace-id> ./scripts/quickstart-two-agents.sh
#
# `maidan init` prints both the admin bearer token and the workspace id. The admin
# token creates the two agents and the channel and thread; each agent then posts
# and reads with its own worker token.
#
# Override the target URL with MAIDAN_URL.
set -euo pipefail

BASE_URL="${MAIDAN_URL:-http://127.0.0.1:8080}"
TOKEN="${MAIDAN_TOKEN:-}"
WORKSPACE_IN="${MAIDAN_WORKSPACE:-}"

for cmd in curl jq; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "missing required command: $cmd" >&2; exit 1; }
done

if [ -z "$TOKEN" ] || [ -z "$WORKSPACE_IN" ]; then
  echo "MAIDAN_TOKEN and MAIDAN_WORKSPACE are required." >&2
  echo "Run 'maidan init --workspace demo' and pass the printed values." >&2
  exit 1
fi
AUTH=(-H "authorization: Bearer ${TOKEN}")
echo "auth: bearer token (member-bound workers)"

post() {
  # post <path> <json-body>  -> response body, as the admin.
  curl -fsS -H 'content-type: application/json' "${AUTH[@]}" -X POST "${BASE_URL}$1" --data "$2"
}
post_as() {
  curl -fsS -H 'content-type: application/json' -H "authorization: Bearer $1" \
    -X POST "${BASE_URL}$2" --data "$3"
}
get_as() {
  curl -fsS -H "authorization: Bearer $1" "${BASE_URL}$2"
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

workspace="$WORKSPACE_IN"

# Each agent is a member with its own worker token, made in one call by the admin.
planner_body=$(post "/workspaces/${workspace}/agents" '{"handle":"planner"}')
reviewer_body=$(post "/workspaces/${workspace}/agents" '{"handle":"reviewer"}')
planner=$(jq -er '.member.id' <<<"$planner_body")
reviewer=$(jq -er '.member.id' <<<"$reviewer_body")
planner_token=$(jq -er '.token.secret' <<<"$planner_body")
reviewer_token=$(jq -er '.token.secret' <<<"$reviewer_body")
channel=$(post "/workspaces/${workspace}/channels" '{"name":"coordination"}' | jq -er '.id')
thread=$(post "/channels/${channel}/threads" '{"title":"launch-plan"}' | jq -er '.id')

# Agent A (planner) posts a request into the shared thread.
post_as "$planner_token" "/threads/${thread}/messages" \
  '{"body":"Reviewer: what is the highest-risk item on the launch checklist?"}' \
  >/dev/null

echo
echo "reviewer reads the shared thread:"
get_as "$reviewer_token" "/threads/${thread}/context" | jq '.messages[] | {author_id, body}'

# Agent B (reviewer) reads the thread above, then replies into the same durable thread.
post_as "$reviewer_token" "/threads/${thread}/messages" \
  '{"body":"Highest risk is first-run onboarding: the quickstart must work from a clean machine."}' \
  >/dev/null

echo
echo "planner reads the reply (both messages are durable shared state):"
get_as "$planner_token" "/threads/${thread}/context" | jq '.messages[] | {author_id, body}'

echo
echo "demo resources:"
printf '  workspace: %s\n  planner:   %s\n  reviewer:  %s\n  channel:   %s\n  thread:    %s\n' \
  "$workspace" "$planner" "$reviewer" "$channel" "$thread"
