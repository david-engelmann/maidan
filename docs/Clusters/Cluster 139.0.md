# Cluster 139.0 — 1:1 direct messages in the console

**Theme:** Surface the already-shipped 1:1 DM API in the `/ui` console — a
new "DMs" tab. The exact parallel to group DMs (136); closes the gap where
group DMs were surfaced but 1:1 DMs were not.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v139.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | `GET /ui/api/workspaces/:wid/dm` (read, `workspace:read`); `POST /ui/api/workspaces/:wid/dm` (open) + `POST /ui/api/dm/:id/messages` (post) on the write router. Reuses `dm::{list_dm_conversations,open_dm_conversation,post_dm_message}`; conversation pane reads via the existing `/ui/api/threads/:tid/messages`. |
| **UI (index.html)** | A `panel-dms` view: open a DM by the other member's ID (actor = signed-in member; self-DM rejected), a refreshable list (each row shows the *other* member), and a conversation pane (select → load, send → post). |

## Non-goals

- A real-time DM stream — refresh-on-demand, consistent with group DMs and
  the rest of the console.
- DM-specific read endpoints in the UI — the conversation reuses
  `/ui/api/threads/:tid/messages` (DMs are thread-backed), so no
  `/ui/api/dm/:id/messages` GET is added.
- A dedicated `/ui/api` DM backend test — the handlers + `/ui/api`
  middleware are each already covered.

## PR ladder (actual)

| # | Title |
|---|--------|
| 139.0.1 | `feat(ui): 1:1 direct messages in the console` (#368) |
| 139.0.retro | `docs(retro): Cluster 139.0 + v139.0.0 tag prep` |

## Exit criteria

- DMs open / list / read / post in the UI; routes wired under `/ui/api`;
  guard green — **met**.
- `v139.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; `fmt`/`clippy` clean. Per the
  UI track's standing limit, JS *behavior* is inspection-verified (no browser).

## References

- [[Retros/Cluster 139.0]]; [[Clusters/Cluster 136.0]] (group-DM parallel);
  `static/index.html` (`loadDms`/`openDm`/`selectDm`/`sendDmMessage`),
  `app.rs`, `dm.rs`.
