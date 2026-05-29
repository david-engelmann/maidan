# Cluster 46.0 — Edit history & message UX

**Theme:** Per-edit body history table and “edited” affordance in `/ui`.

## Problem

`PATCH /messages/:id` set `edited_at` but left no durable before/after trail and the UI did not surface edits.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_message_edits` migration; record on body change; `list_message_edits` |
| HTTP | `GET /messages/:id/edits`, `GET /ui/api/messages/:mid/edits` |
| UI | “edited” marker on messages; edit history panel with before → after |

## Tag

`v46.0.0`

See [[Clusters/Product Ladder 35+]] Phase III.
