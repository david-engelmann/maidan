# Cluster 38.0 — MCP resource fan-out complete

**Theme:** Emit `notifications/resources/updated` for every HTTP mutation that changes readable MCP resources.

## Problem

`v33.0.0` fans out on tombstone + FSM only. Agents subscribed via MCP miss updates when messages are
edited, workspaces purged, votes cast, or mentions created over HTTP.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Server | `publish_resource_uris` on `edit_message`, `purge_workspace`, `cast_vote`, `create_mention` |
| MCP | Reuse `resource_updates` helpers |
| Tests | E2E per path with `GET /mcp/notifications` or streamable subscriber |
| Docs | Capabilities + Remaining Work gap closure |

## Out of scope

- New resource URI schemes
- WebSocket fan-out changes

## Tag

`v38.0.0`

## Depends on

Cluster 37 (`v37.0.0`).

See [[Clusters/Product Ladder 35+]] Phase I.
