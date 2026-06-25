# Cluster 139.0 retro — 1:1 direct messages in the console

> Tag **`v139.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`/ui/api` DM routes**: `GET /ui/api/workspaces/:wid/dm` (read,
  `workspace:read`); `POST /ui/api/workspaces/:wid/dm` (open) +
  `POST /ui/api/dm/:id/messages` (post) on the write router — reusing the
  existing tested `dm::{list_dm_conversations,open_dm_conversation,post_dm_message}`.
- **"DMs" tab in `index.html`** (`panel-dms`): open a 1:1 DM by the other
  member's ID (actor is the signed-in member; self-DM rejected), refresh the
  list (each row shows the *other* member), select a conversation, read its
  messages (via the existing `/ui/api/threads/:tid/messages`), and post as
  the actor.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Real-time DM stream | Refresh-on-demand matches group DMs + the rest of the console. |
| n/a | `/ui/api/dm/:id/messages` GET | DMs are thread-backed; the conversation pane reuses the thread-messages read route. |
| n/a | `/ui/api` DM backend test | Handlers + `/ui/api` middleware are each already covered. |

## Surprises

- **`DmConversation` has no title** and stores a canonicalized
  `member_low_id`/`member_high_id` pair (so a DM is order-independent), so
  the list derives the "other" participant by comparing each id against the
  actor — unlike group DMs, which carry an explicit title.
- **`open_dm_conversation` is gated by `workspace:read`, not a write cap** —
  opening a DM is idempotent (returns the existing conversation for the
  pair), so it reads as a lookup; it still sits on the write router because
  it's a POST and the write session grants `workspace:read` anyway.

## Decisions

- **A separate "DMs" tab** rather than folding 1:1 into the Group DMs tab —
  keeps each surface single-purpose and mirrors the group-DM panel exactly.
- **Reuse handlers under `/ui/api`** (as with group DMs) — no new backend
  logic; the conversation reuses the thread-messages read path.

## Capability table extension

| Capability | Where |
|------------|-------|
| Open / list / read / post 1:1 DMs in the `/ui` console | `static/index.html`, `/ui/api/workspaces/:wid/dm`, `/ui/api/dm/:id/messages` |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the dm e2e covers the API.

## Forward look

DMs (139) + group DMs (136) now both surface in the `/ui`. Remaining
unsurfaced backend collaboration features include presence (`presence.rs`)
and slash commands (`slash_commands.rs`); reassess against [[Open Work]]
before opening 140.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
