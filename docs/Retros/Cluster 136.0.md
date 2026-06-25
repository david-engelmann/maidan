# Cluster 136.0 retro — Group DMs in the operator console

> Tag **`v136.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`/ui/api` group-DM routes** (session **or** bearer): `GET`/`POST`
  `/ui/api/workspaces/:wid/group-dms` (list by member / open),
  `GET /ui/api/group-dms/:id`, `POST /ui/api/group-dms/:id/messages` —
  reusing the existing tested
  `group_dm::{list_group_dms,open_group_dm,get_group_dm,post_group_dm_message}`
  handlers.
- **`panel-group-dms` view in `index.html`**: open a group DM (member ids +
  optional title; the actor is auto-included, ≥2 members enforced), refresh
  the list (GET by `member_id = authorId()`), select a conversation, read
  its messages (via the existing `/ui/api/threads/:tid/messages`), and post
  as the actor.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Real-time group-DM stream in the UI | Refresh-on-demand matches the rest of the console; a WS-backed live pane can follow. |
| Future | Membership edits (add/remove) on an existing group DM | Open + post is the operator surface; the backend already supports membership, the UI can expose it later. |
| n/a | `/ui/api` group-DM backend test | Handlers + `/ui/api` middleware are each already covered; new routes wire tested pieces. |

## Surprises

- **No new read endpoint needed for the conversation pane.** Group DMs are
  thread-backed (`GroupDmConversation.thread_id`), so messages read straight
  through the existing `/ui/api/threads/:tid/messages` route — the new view
  only added the *list/open/post* surface.

## Decisions

- **A new tab/view over a per-message affordance.** Unlike reactions/pins,
  group DMs aren't attached to a thread the operator is already viewing, so
  they need their own open + list + conversation surface.
- **Reuse handlers under `/ui/api`** (as with reactions/pins) — no new
  backend logic; writes go through `apiWritePath`/`requireAuthForWrite`,
  reads through `uiReadPath`.

## Capability table extension

| Capability | Where |
|------------|-------|
| Open / list / read / post group DMs in the `/ui` console | `static/index.html`, `/ui/api/.../group-dms` |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the group-DM e2e covers the API.

## Forward look

Next UI cluster: **137** operator console (surface deliveries/DLQ, reindex
jobs, and the global audit from 132 in the `/ui`).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
