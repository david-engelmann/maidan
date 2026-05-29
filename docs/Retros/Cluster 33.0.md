# Cluster 33.0 retro — MCP resource fan-out (HTTP)

> Tag **`v33.0.0`**.

## What shipped

- `McpServer::publish_resource_uris` shared by tools and HTTP.
- `DELETE /messages/:id` and `POST /threads/:id/transition` emit resource notifications when subscribed.

## Forward look

Cluster 34: `Mcp-Session-Id` on streamable HTTP.
