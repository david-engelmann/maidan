# Cluster 45.0 — Admin console

**Theme:** Audit viewer, purge confirm, peer admin, token mint/revoke in `/ui`.

## Problem

Destructive and federation operations required curl; tokens tab could mint but not revoke with capabilities.

## Scope

| Layer | Deliverable |
|-------|-------------|
| UI | Admin tab: audit log, purge confirmation, federation peers, token capabilities + revoke |
| API | `GET /ui/api/workspaces/:wid/audit`, `GET /ui/api/workspaces/:wid/peers` |

## Tag

`v45.0.0`

See [[Clusters/Product Ladder 35+]] Phase III.
