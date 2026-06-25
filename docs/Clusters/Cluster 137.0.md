# Cluster 137.0 — Deliveries & DLQ in the operator console

**Theme:** Surface webhook + automation deliveries (including the
dead-letter queue) for the current workspace in the `/ui` console, with a
one-click replay. First feature of the "Operator" tab.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v137.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | `GET /ui/api/workspaces/:wid/deliveries` (read, `workspace:read`) → `delivery_ops::list_deliveries`; `POST /ui/api/workspaces/:wid/deliveries/:did/replay` (write, `workspace:write`) → `delivery_ops::replay_delivery`. |
| **UI (index.html)** | A new "Operator" tab / `panel-operator`: status filter (pending / quarantined / delivered) + kind filter (all / webhook / automation) + refreshable list; each row shows kind, id, attempts, target URL, last error, timestamps, with a Replay button. |

## Why this scope (and what was deferred)

The original "operator console" idea bundled three things — deliveries/DLQ,
reindex jobs, and the global audit. Only deliveries/DLQ map onto the
**operator-session** capabilities (`workspace:read` for list, `workspace:write`
for replay), so it works on a plain login. The other two need **elevated
bearer tokens**:

- **Global audit** (`GET /operator/audit`) requires `audit:read-global`.
- **Global reindex** (`POST /operator/reindex-embeddings`) requires
  `token:admin` (workspace-scoped reindex needs `workspace:write`).

Shipping those under `/ui/api` would 403 on a normal session, so they're
deferred to a follow-up (138) rather than degrade the tab. This keeps 137 a
tight, fully-session-capable feature.

## Non-goals

- Rendering automation auth-header fields (`header_name`/`header_value`) —
  deliberately omitted; they can carry secrets.
- A live-updating delivery stream — refresh-on-demand, like the rest of the
  console.
- A dedicated `/ui/api` delivery backend test — the handlers + `/ui/api`
  middleware are each already covered.

## PR ladder (actual)

| # | Title |
|---|--------|
| 137.0.1 | `feat(ui): deliveries & DLQ operator view with replay` (#364) |
| 137.0.retro | `docs(retro): Cluster 137.0 + v137.0.0 tag prep` |

## Exit criteria

- Deliveries list + filter + replay in the UI; routes wired under `/ui/api`;
  guard green — **met**.
- `v137.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; `fmt`/`clippy` clean. Per the
  UI track's standing limit, JS *behavior* is inspection-verified (no browser).

## References

- [[Retros/Cluster 137.0]]; `static/index.html`
  (`loadDeliveries`/`renderDeliveries`/`replayDelivery`), `app.rs`,
  `delivery_ops.rs`.
