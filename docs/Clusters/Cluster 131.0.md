# Cluster 131.0 — Delivery-unification verification-close

**Theme:** Resolve the long-standing "unify webhook + automation delivery"
backlog item by verifying it against code — it is substantially addressed — and
declining a risky storage-table migration that would add no functional value.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v131.0.0`**, no new
gate tag. Docs-only verification cluster (cf. Cluster 127).

---

## Finding (verified at v130)

| Layer | State |
|-------|-------|
| Signing | **Shared** — `automation_delivery` reuses `webhooks::sign_payload`. |
| Backoff | **Shared** — `automation_delivery::backoff` = `webhooks::delivery_backoff`. |
| Operator API | **Unified** — `delivery_ops` exposes `OperatorDelivery::{Webhook,Automation}` over both (list/get/replay). |
| Storage | **Intentionally separate** — `maidan_webhook_deliveries` (FK → webhook subscriptions) vs `maidan_automation_deliveries` (FK → slash/fsm configs). Identical retry/quarantine *schema*, different *referents*. |

## Decision

**Do not merge the storage tables.** A merge means a data migration across two
FK-distinct tables, a kind-discriminator column, and rewriting every store
method — high risk — for zero functional gain (the user-facing and logic layers
are already unified). The separation is a deliberate, FK-driven design, not an
oversight. Closed as **substantially-addressed**.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Docs** | Close the item in `Remaining Work.md` §3 + `Open Work.md` with the evidence; record the storage-separation rationale. |

## Non-goals

- The storage-table migration (declined, see Decision).
- Refactoring the two working+tested relay loops into a shared generic — marginal
  DRY gain for regression risk on delivery code.

## PR ladder (actual)

| # | Title |
|---|--------|
| 131.0.1 + retro | `docs(retro): Cluster 131.0 — delivery-unification close + v131.0.0 tag prep` |

## Exit criteria

- The unify-delivery backlog item is resolved with a documented rationale — **met**.
- `v131.0.0` tagged.

## References

- [[Retros/Cluster 131.0]]; `automation_delivery.rs`, `webhooks.rs`, `delivery_ops.rs`
