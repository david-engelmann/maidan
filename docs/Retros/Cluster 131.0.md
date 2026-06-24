# Cluster 131.0 retro — Delivery-unification verification-close

> Tag **`v131.0.0`**. Phase XXIV (post-gate hardening). Docs-only. No new gate
> tag. Authorized as a feature cluster, resolved as a verification-close once the
> code showed the work was substantially done.

## What shipped

- **Closed the "unify webhook + automation delivery" backlog item** in
  `Remaining Work.md` §3 and `Open Work.md`, with evidence: signing + backoff are
  already shared (`automation_delivery` reuses `webhooks::sign_payload` /
  `delivery_backoff`); the operator API is already unified (`OperatorDelivery`).
  The two storage tables remain separate **by design** (distinct foreign keys),
  and the rationale is now recorded so no future planner re-attempts the merge.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Declined | Merge `maidan_webhook_deliveries` + `maidan_automation_deliveries` | Risky data migration (FK-distinct tables, kind discriminator, rewrite all store methods) for zero functional gain — the logic + operator API are already unified. |
| Declined | Generic relay-loop refactor | Marginal DRY gain vs regression risk on working, tested delivery code. |

## Surprises

- **"Unify" was ~80% done already.** The backlog framed this as separate queues
  needing merging, but the divergence had quietly shrunk to just the storage
  tables — signing, backoff, and the operator API converged across earlier
  clusters (68/80) without the backlog noting it. Another reconciliation win.

## Decisions

- **Decline the migration; close with rationale.** When an authorized item turns
  out to be substantially addressed and the remainder is risky-low-value, the
  right deliverable is a documented close, not a needless migration. (Same
  judgment as the Cluster 127 phantom-gap reconciliation.)
- **Storage separation is a feature, not debt.** Distinct foreign keys to
  distinct referents (subscriptions vs slash/fsm configs) is correct modeling;
  merging would force a nullable-FK or polymorphic-FK anti-pattern.

## Capability table extension

| Capability | Where |
|------------|-------|
| Webhook+automation delivery unified at logic + operator-API layers (storage intentionally separate) | `automation_delivery.rs`, `webhooks.rs`, `delivery_ops.rs` |

## Risks identified + still open

- None introduced (docs-only).

## Forward look

Last remaining authorized item: **Cluster 132** — expose the global
cross-workspace admin audit query API (the audit data exists per workspace; the
query API is not yet exposed). That's a genuine backend feature.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
