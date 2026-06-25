# Cluster 137.0 retro — Deliveries & DLQ in the operator console

> Tag **`v137.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`/ui/api` delivery routes**: `GET /ui/api/workspaces/:wid/deliveries`
  (read, `workspace:read`) and
  `POST /ui/api/workspaces/:wid/deliveries/:did/replay` (write,
  `workspace:write`), reusing the existing tested
  `delivery_ops::{list_deliveries,replay_delivery}` handlers.
- **"Operator" tab in `index.html`** (`panel-operator`): a status filter
  (pending / quarantined / delivered), a kind filter (all / webhook /
  automation), and a refreshable list. Each delivery row renders kind, id,
  attempts, target URL, last error, and timestamps, with a Replay button
  that re-attempts the delivery (`POST .../replay?kind=`) and refreshes.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| **138** | Global audit (`/operator/audit`) in the UI | Needs `audit:read-global`, which the operator session doesn't carry; bearer-only. |
| **138** | Reindex-embeddings controls in the UI | Global reindex needs `token:admin`; workspace reindex needs `workspace:write` (session-capable) but pairs naturally with the global controls. |
| n/a | Automation auth-header fields in the row | `header_name`/`header_value` can carry secrets — deliberately not rendered. |
| n/a | `/ui/api` delivery backend test | Handlers + `/ui/api` middleware are each already covered; new routes wire tested pieces. |

## Surprises

- **`OperatorDelivery` is internally tagged** (`#[serde(tag = "kind")]`,
  snake_case), so each JSON row carries `kind: "webhook"|"automation"` plus
  the inner fields flattened — one render path handles both transports.
- **The capabilities lined up exactly.** Delivery list is `workspace:read`
  and replay is `workspace:write`, which are precisely what the read- and
  write-session middlewares grant — so the whole view works on a plain
  operator login with no bearer token, unlike the audit/reindex controls.

## Decisions

- **Scope to the session-capable surface.** Bundling audit + reindex would
  have meant shipping controls that 403 on a normal session; split those to
  138 (bearer-gated) and keep 137 fully usable on a login.
- **Reuse handlers under `/ui/api`** (as with reactions/pins/group-DMs) — no
  new backend logic.
- **Never render `header_value`.** Avoids leaking automation auth secrets in
  the operator surface.

## Capability table extension

| Capability | Where |
|------------|-------|
| List + replay webhook/automation deliveries (incl. DLQ) in `/ui` | `static/index.html`, `/ui/api/workspaces/:wid/deliveries[/:did/replay]` |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the delivery-ops e2e covers the API.

## Forward look

Next: **138** — bearer-gated operator controls (global audit + reindex
embeddings) to complete the operator console.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
