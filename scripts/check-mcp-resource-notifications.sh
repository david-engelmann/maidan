#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Cluster 71: HTTP mutations that should fan out MCP resource notifications must
# call into McpServer queue_resource_updates (grep parity checklist).

ROUTES="crates/maidan-server/src/routes.rs"
MCP="crates/maidan-mcp/src/tools.rs"

for sym in post_message edit_message pin_message unpin_message add_reference; do
  if ! rg -q "queue_resource_updates" "$MCP" 2>/dev/null; then
    echo "missing queue_resource_updates in $MCP"
    exit 1
  fi
done

if ! rg -q 'resources/subscribe' "$MCP"; then
  echo "MCP resources/subscribe not found"
  exit 1
fi

if ! rg -q 'mcp/notifications' crates/maidan-server/src/app.rs; then
  echo "GET /mcp/notifications route missing"
  exit 1
fi

echo "mcp resource notification parity checklist ok"
