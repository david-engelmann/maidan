# Cluster 70.0 — Vault truth pass

**Theme:** Finish Cluster **59** documentation debt — the vault must describe **`v67.0.0`**, not pre–`maidan-2.0` stubs.

## Problem

[[Architecture]] still opens with “state at `v0.4.0`”. [[Remaining Work]] and [[Open Work]] list items shipped in Clusters **35–67** (Helm stack, DMs, pins, outbox replay, streamable mux, etc.). Integrators and agents inherit a false mental model.

Cluster **59** shipped [[Agent Integration]] and contract CI but deferred the vault sweep.

## Scope

| Doc | Action |
|-----|--------|
| [[Architecture]] | Rewrite snapshot for agent substrate: transports, apps, automation, context, quotas |
| [[Remaining Work]] | Reconcile §1 partials and §3 deferrals against `main` |
| [[Open Work]] | Update “latest tag”, remove shipped rows, link [[Clusters/Product Ladder 68+]] |
| `README.md` (repo root) | Current tags, link Agent Integration, drop stale v28-only pitch |
| [[README]] (vault index) | Point active cluster → **68**; add 59+ and 68+ ladder links |
| [[Roadmap]] | Already points at 67 close; add **68+ active** pointer |

## Non-goals

- mdBook content generation (unless trivial link fixes).
- New features.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 70.0.1 | `docs: refresh Architecture for v67 agent substrate` |
| 70.0.2 | `docs: reconcile Remaining Work and Open Work` |
| 70.0.3 | `docs: update root README and vault index` |
| 70.0.retro | `docs(retro): Cluster 70.0 + v70.0.0 tag prep` |

## Exit criteria

- No doc claims “pins not implemented” / “A2A stub” / “outbox replay absent” where code exists.
- `v70.0.0` tagged after retro (docs-only cluster is valid).

## References

- [[Clusters/Product Ladder 68+]], [[Clusters/Product Ladder 59+]], [[Agent Integration]]
