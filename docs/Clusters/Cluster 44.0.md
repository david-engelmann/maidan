# Cluster 44.0 — UI collaboration flows

**Theme:** Create channel/thread, post/edit message, artifact upload, faceted search in `/ui`.

## Problem

UI v2 could browse channels and tail events but could not drive the collaboration loop without curl.

## Scope

| Layer | Deliverable |
|-------|-------------|
| UI | Thread sidebar, collab panel (compose, edit, artifact upload), faceted search |
| API | `GET /ui/api/channels/:cid/threads`, `.../threads/:tid/messages`, `.../search` (session or bearer) |
| Writes | Browser uses bearer on existing protected routes |

## Tag

`v44.0.0`

See [[Clusters/Product Ladder 35+]] Phase III.
