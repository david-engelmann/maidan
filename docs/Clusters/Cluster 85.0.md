# Cluster 85.0 — sqlite-vec optional

**Theme:** sqlite-vec optional.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XV · tag **`v85.0.0`**.

**Predecessor:** Cluster **75** semantic runbook.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XV row for cluster **85**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Feature-gated HNSW on SQLite via optional `sqlite-vec`; CI proves linkage or documents opt-out. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Default-on sqlite-vec in all builds.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 85.0.1 feat(search): optional sqlite-vec feature flag |
| 85.0.2 ci: sqlite-vec linkage job |
| 85.0.retro docs(retro): Cluster 85.0 + v85.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v85.0.0`** tagged after retro.

---

## Implementation plan (post-84)

**Current state:** `maidan-search` defaults to the `sqlite-vec` feature; semantic search already uses `vec_distance_cosine` when the extension loads (`v48.0.0`).

**Cluster 85 deliverables:**

| Step | Work |
|------|------|
| 85.0.1 | Remove `sqlite-vec` from `default` features; add `default = []` and document `cargo build -p maidan-search --features sqlite-vec`. |
| 85.0.2 | CI job `sqlite-vec` (or matrix leg): build + test with `--features sqlite-vec`; workspace default job builds without it. |
| 85.0.3 | `semantic_search` on SQLite without feature: keep brute-force fallback; with feature: SQL HNSW path unchanged. |
| 85.0.4 | [[Production]] + [[Agent Integration]]: when to enable the feature; opt-out for minimal binaries. |

**Exit check:** CI proves linkage with feature on; default workspace build does not require `libsqlite3-sys` / `sqlite_vec0`.

**Risk:** Downstream crates that assumed default `sqlite-vec` need explicit `--features` in their dev-deps or CI.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 84.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]
