# Cluster 41.0 — Reactions & pins

**Theme:** Emoji reactions alongside votes; pin/unpin message API + events.

## Problem

Votes cover approval semantics but not lightweight emoji reactions or thread pins.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_reactions`, `maidan_pins` |
| Server | Message reactions CRUD; thread pin/unpin/list |
| Events | `reaction_added`, `reaction_removed`, `message_pinned`, `message_unpinned` |
| MCP | `add_reaction`, `remove_reaction`, `list_reactions`, `pin_message`, `unpin_message`, `list_pins` |
| Tests | Store + HTTP e2e |

## Tag

`v41.0.0`

See [[Clusters/Product Ladder 35+]] Phase II.
