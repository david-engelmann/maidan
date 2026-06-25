# Cluster 136.0 — Group DMs in the operator console

**Theme:** Open / list / read / post group DMs from the `/ui` console,
over the already-shipped group-DM API. Same `/ui/api` mount pattern as
reactions (134) and pins (135), but a new *view* rather than a per-message
affordance.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v136.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | Under `/ui/api` (session **or** bearer): `GET`/`POST` `/ui/api/workspaces/:wid/group-dms` (list / open), `GET /ui/api/group-dms/:id`, `POST /ui/api/group-dms/:id/messages`. Reuses `group_dm::{list_group_dms,open_group_dm,get_group_dm,post_group_dm_message}`; reads thread messages via the existing `/ui/api/threads/:tid/messages`. |
| **UI (index.html)** | A `panel-group-dms` view: open-form (comma-separated member ids + optional title), a refreshable list (GET by `member_id = authorId()`), and a conversation pane (select → load messages, send → POST as actor). |

## Non-goals

- A real-time group-DM stream in the UI — the list/conversation panes are
  refresh-on-demand, consistent with the rest of the console (events, etc.).
- Member management of an existing group DM (add/remove) — open + post is
  the operator surface; membership edits can follow if wanted.
- A dedicated `/ui/api` group-DM backend test — the handlers and the
  `/ui/api` middleware are each already covered; this is pure wiring.

## PR ladder (actual)

| # | Title |
|---|--------|
| 136.0.1 | `feat(ui): group DMs in the operator console` (#362) |
| 136.0.retro | `docs(retro): Cluster 136.0 + v136.0.0 tag prep` |

## Exit criteria

- Group DMs open / list / read / post in the UI; routes wired under
  `/ui/api`; guard green — **met**.
- `v136.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; `fmt`/`clippy` clean. Per the
  UI track's standing limit, JS *behavior* is inspection-verified (no browser).

## References

- [[Retros/Cluster 136.0]]; `static/index.html`
  (`loadGroupDms`/`openGroupDm`/`selectGroupDm`/`sendGroupDmMessage`),
  `app.rs`, `group_dm.rs`.
