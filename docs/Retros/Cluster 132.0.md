# Cluster 132.0 retro — Global cross-workspace admin audit query API

> Tag **`v132.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. **Final
> cluster of the planned 127–132 sweep.**

## What shipped

- **`GET /operator/audit`** — cross-workspace audit query, gated by the new
  global capability **`audit:read-global`**. Returns `store.list_audit(limit)`
  (clamped 1..=500), recent-first, across all workspaces. Intentionally **not**
  `ensure_workspace`-gated (it spans workspaces); the capability is the gate.
- **OpenAPI + capability-map wiring** so the route is a properly-classified
  bearer op — the Cluster 121 bidirectional contract stays green.
- **Tests**: the table-driven `http_capability_matrix_e2e` covers denial (a token
  lacking the capability → 403); `operator_audit_e2e` covers allow (a token with
  it gets every audit event back, auth enabled).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Admin *console* / UI | Backend query API only; UI is product work (§4). |
| Future | Richer audit filters / pagination | `limit` only for now; `list_audit` is recent-first. |
| Out of scope | Org/global-admin principal model | Not needed — the capability is the gate. |

## Surprises

- **The hard part (cross-workspace auth) dissolved.** A "global admin" query
  seemed to need a super-admin principal Maidan doesn't have — but a token-held
  *capability* + skipping `ensure_workspace` is a clean, sufficient gate, with
  `federation:admin` as the existing precedent. No org model required.
- **The data + store query already existed.** `Store::list_audit` was there
  (cross-workspace); this cluster was wiring + auth + contract, not new storage —
  consistent with the sweep's theme that most "gaps" are thinner than the backlog
  implied.
- **The capability-map contract has teeth.** Adding a bearer route forced touching
  the OpenAPI ApiDoc + `http-capability-map.json` + the matrix test's deny-set —
  exactly the cross-checks Cluster 121 was built to enforce. The new route could
  not ship half-wired.

## Decisions

- **Capability-as-gate, no org model.** Authorize by an explicit global
  capability rather than inventing a super-admin/org hierarchy (out of scope).
- **Backend API, not a console.** Deliver the queryable endpoint; leave UI to
  product work.

## Capability table extension

| Capability | Where |
|------------|-------|
| `GET /operator/audit` global cross-workspace audit (cap `audit:read-global`) | `routes/workspace.rs::list_global_audit`, `capability.rs` |

## Risks identified + still open

- **Grant discipline.** `audit:read-global` is powerful (reads all workspaces'
  audit); operators must mint it deliberately. It's not in `default_minted`.

## Forward look

**The planned 127–132 sweep is complete.** 127 reconciled the backlog; 128–130
hardened delivery, buffers/errors, and coverage; 131 closed delivery-unification;
132 shipped the global audit API. Remaining backlog is product/UI (needs product
decisions) + explicitly out-of-scope items. A clean stopping point.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
