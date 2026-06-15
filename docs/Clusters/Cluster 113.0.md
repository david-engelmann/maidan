# Cluster 113.0 — Backend parity harness

**Theme:** Keep the Postgres and SQLite backends in lockstep — both structurally (no migration/module exists for only one) and behaviorally (same operations, same user-visible results).

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXI · tag **`v113.0.0`**.

**Predecessor:** Dual-backend store from Cluster A; shared assertion harness (`tests/common/mod.rs`, `dialect_parity.rs`).

---

## Problem

Maidan ships two `Store` implementations behind one trait (`src/postgres/`, `src/sqlite/`) with two migration trees (`migrations/postgres`, `migrations/sqlite`). The shared assertion suite (`run_full_roundtrip`) and the cross-dialect identity test (`dialect_parity`) prove *behavioral* parity for the operations they cover — but nothing caught *structural* drift: a migration or store module added to one backend and forgotten on the other would pass CI. As the ladder adds features, that drift risk only grows.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Static guard** | A Docker-free test asserting every migration slug and every store module exists for both backends, modulo a documented allowlist; runs in the required `unit tests` job. |
| **Behavioral** | Broaden the cross-dialect identity snapshot to cover more surface (FSM transition, edit, reaction) so the two backends are held to identical results there too. |

## Non-goals

- Column-level / type-level schema diffing — the dialects intentionally differ (`JSONB` vs `TEXT`, `TIMESTAMPTZ` vs `TEXT`); type parity is asserted behaviorally, not structurally.
- A new required CI job — the guard rides in the existing `unit tests` check.
- Changing any backend behavior — tests only.

## PR ladder (actual)

| # | Title |
|---|--------|
| 113.0.1 | `test(store): backend parity guard + broadened cross-dialect snapshot` (#312) |
| 113.0.retro | `docs(retro): Cluster 113.0 + v113.0.0 tag prep` |

## Exit criteria

- A shared assertion suite both backends must pass — **met** (`run_full_roundtrip`, `dialect_parity`, now broadened).
- A CI guard that `migrations/postgres` ↔ `migrations/sqlite` and store modules stay in lockstep — **met** (`backend_parity.rs`, allowlisted).
- `v113.0.0` tagged after retro.

## Ordering & risks

- **After [[Clusters/Cluster 111.0]] / [[Clusters/Cluster 112.0]]** — the pure-logic test clusters; this one adds the cross-backend guard.
- **Risk — index vs slug:** the backends' migration numbering already diverged, so the guard compares slugs (feature names), not indices.
- **Risk — allowlist rot:** every allowlist entry requires an in-code rationale, reviewed in PR.

## References

- [[Clusters/Product Ladder 102+]] Phase XXI
- [[Retros/Cluster 113.0]], [[Architecture]], [[Decisions]]
