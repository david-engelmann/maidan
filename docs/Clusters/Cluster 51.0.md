# Cluster 51.0 — Slash commands

**Theme:** `/command` parsing on message post with pluggable handlers.

## Problem

Humans and agents need quick imperative triggers in threads without
building a full automation UI.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Router | `parse_slash_command` in `maidan-router` |
| Store | `maidan_slash_commands` |
| HTTP | `POST/GET/DELETE /workspaces/:wid/slash-commands` |
| Dispatch | On `post_message`, invoke `http` or `mcp_tool` handler; stash result in message metadata |
| MCP | `register_slash_command`, `list_slash_commands` tools |

## Tag

`v51.0.0`

See [[Clusters/Product Ladder 35+]] Phase V.
