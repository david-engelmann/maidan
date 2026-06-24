# Cluster 132.0 — Global cross-workspace admin audit query API

**Theme:** Expose the existing cross-workspace audit query (`Store::list_audit`)
over HTTP behind a new global capability — the one genuinely-buildable item from
the Slack-parity "global admin console" gap.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v132.0.0`**, no new
gate tag. Final cluster of the planned 127–132 sweep.

**Predecessor:** workspace audit (01), the unified operator API (80), the
capability-map contract (121).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Capability** | `audit:read-global` (`AUDIT_READ_GLOBAL`) — global, not workspace-scoped (cf. `federation:admin`); added to the known set. |
| **Route** | `GET /operator/audit?limit=` → `list_global_audit`: requires the capability, **not** `ensure_workspace`-gated, returns `store.list_audit(limit)` (clamped 1..=500). |
| **Contract** | OpenAPI path stub + registration; `http-capability-map.json` entry (keeps the 121 bidirectional contract green). |
| **Tests** | Denial via the table-driven `http_capability_matrix_e2e` (new deny-set arm); allow via `operator_audit_e2e`. |

## Why no org model is needed

A cross-workspace query usually implies a super-admin principal, which Maidan
lacks (org hierarchy is out of scope). The capability *is* the gate: a token
explicitly minted with `audit:read-global` is authorized; the route simply
skips `ensure_workspace`. Granting that capability is an operator decision. No
schema or auth-model change beyond the capability string.

## Non-goals

- An admin *console* / UI — this is the backend query API only.
- Filtering/pagination beyond `limit` — `list_audit` returns recent-first; richer
  filters can follow if needed.
- An org/global-admin principal model (out of scope).

## PR ladder (actual)

| # | Title |
|---|--------|
| 132.0.1 | `feat(operator): global cross-workspace audit query API` (#354) |
| 132.0.retro | `docs(retro): Cluster 132.0 + v132.0.0 tag prep` |

## Exit criteria

- `GET /operator/audit` returns cross-workspace audit, gated by the new
  capability; denial + allow both tested; 121 cap-map contract green — **met**.
- `v132.0.0` tagged after retro.

## References

- [[Retros/Cluster 132.0]]; `routes/workspace.rs::list_global_audit`, `capability.rs`
- [[Remaining Work]] §4 (the admin-audit-API exception)
