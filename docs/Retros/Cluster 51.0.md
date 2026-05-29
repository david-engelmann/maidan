# Cluster 51.0 retro — Slash commands

> Tag **`v51.0.0`**.

## What shipped

- `parse_slash_command` (`/name args`) in `maidan-router`.
- `maidan_slash_commands` table + CRUD on `/workspaces/:wid/slash-commands`.
- `post_message` dispatches registered commands to signed HTTP endpoints or MCP tools; results land in `message.metadata.slash_*`.
- MCP tools `register_slash_command` and `list_slash_commands`.

## What was deferred

- Ephemeral-only responses (Slack-style `response_type`).
- UI autocomplete for registered commands.
- Slash commands in DM threads with distinct UX.

## Forward look

Cluster **52**: FSM automation hooks on `ThreadStateChanged`.
