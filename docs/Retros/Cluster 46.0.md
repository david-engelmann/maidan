# Cluster 46.0 retro — Edit history & message UX

> Tag **`v46.0.0`**.

## What shipped

- `maidan_message_edits` table (Postgres + SQLite migration 19/17).
- `Store::edit_message` records body diffs; `list_message_edits`.
- HTTP + `/ui/api` read routes; UI v5 edit history panel and “edited” labels.

## What was deferred

- Metadata-only edit rows in history (body unchanged → no row).
- MCP `list_message_edits` tool.

## Forward look

Cluster **47** — per-model embedding tables (Phase IV).
