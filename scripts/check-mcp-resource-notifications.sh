#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Cluster 71: HTTP mutations fan out via publish_resource_uris; MCP tools/call via
# queue_resource_updates on McpServer (grep parity checklist).

ROUTES="crates/maidan-server/src/routes.rs"
TOOLS="crates/maidan-mcp/src/tools.rs"
SERVER="crates/maidan-mcp/src/server.rs"

if ! grep -qF "queue_resource_updates" "$SERVER"; then
  echo "missing queue_resource_updates in $SERVER"
  exit 1
fi

if ! grep -qF "publish_resource_uris" "$ROUTES"; then
  echo "missing publish_resource_uris in $ROUTES"
  exit 1
fi

for sym in post_message edit_message pin_message unpin_message add_reference; do
  if ! grep -qF "\"$sym\"" "$TOOLS"; then
    echo "missing MCP tool $sym in $TOOLS"
    exit 1
  fi
done

for handler in post_message edit_message pin_message unpin_message create_reference; do
  if ! grep -qF "fn $handler" "$ROUTES"; then
    echo "missing HTTP handler $handler in $ROUTES"
    exit 1
  fi
done

if ! grep -qF "resources/subscribe" "$SERVER"; then
  echo "MCP resources/subscribe not found in $SERVER"
  exit 1
fi

if ! grep -qF "mcp/notifications" crates/maidan-server/src/app.rs; then
  echo "GET /mcp/notifications route missing"
  exit 1
fi

echo "mcp resource notification parity checklist ok"
