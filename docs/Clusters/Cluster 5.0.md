# Cluster 5.0 — Coverage & search quality

Cluster 4.0 closed subscriber continuity at **`v4.0.0`**. CI still enforces a
~9% line-coverage floor from **`v3.0.0`**, but overall depth remains low and
semantic search can return stale vectors indexed under a different embedding
model because `semantic_search` does not filter on `maidan_message_embeddings.model`.

> **Goal:** Raise measured line coverage with targeted tests and a bumped
> `COVERAGE_MIN_LINES`; publish coverage to Codecov when configured; make
> semantic search model-aware and document rank semantics across backends.
>
> **Target tag:** `v5.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 5.0.1     | `test: targeted coverage uplift + raise COVERAGE_MIN_LINES`            | TBD   |
| 5.0.2     | `ci: optional Codecov upload from llvm-cov lcov artifact`              | TBD   |
| 5.0.3     | `feat(maidan-search): bind semantic search to active embedding model`  | TBD   |
| 5.0.4     | `docs: search rank semantics + embedding model in OpenAPI/Production` | TBD   |
| 5.0.retro | `docs(retro): Cluster 5.0 retrospective + v5.0.0 tag prep`            | TBD   |

## Order

1. **5.0.1** — run `cargo llvm-cov` on green `main`; add focused tests in the
   lowest-coverage, highest-risk crates (prioritize `maidan-server` handlers,
   `maidan-auth`, `maidan-bus`, `maidan-search` edge paths — not blanket
   `unwrap` tests). Re-measure and raise `COVERAGE_MIN_LINES` in
   `.github/workflows/ci.yml` to slightly below the new `main` measurement;
   update [[Operations]] with run id and bump policy (same pattern as 3.0.3).
2. **5.0.2** — upload `lcov.info` from the existing coverage job to Codecov
   when `CODECOV_TOKEN` is set (fork PRs skip gracefully). Document token setup
   in [[Operations]]; optional README badge. Do not fail PRs when token is absent.
3. **5.0.3** — filter Postgres `semantic_search` on
   `e.model = <active provider model_name>`; include `embedding_model` on
   [`SearchHit`](../../crates/maidan-search/src/hit.rs) (HTTP + MCP + OpenAPI).
   Expose active `model` / `dimension` on `/health` (or readiness) when
   embeddings are enabled. Integration tests: mixed-model rows in DB, query
   returns only current model.
4. **5.0.4** — document lexical vs semantic vs SQLite `rank` ranges in
   [[Architecture]] / [[Production]]; extend OpenAPI `SearchHit` description;
   note that ranks are not comparable across `mode=lexical` and `mode=semantic`.
5. **5.0.retro** + `v5.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + raised coverage floor).
- `COVERAGE_MIN_LINES` reflects a fresh green `main` measurement (documented in
  [[Operations]]).
- Codecov receives `lcov.info` on `main` when `CODECOV_TOKEN` is configured.
- Postgres `mode=semantic` ignores embeddings stored under a different `model`
  than the configured provider; hits expose which model matched.
- [[Retros/README]] includes Cluster 5.0; `v5.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Coverage bump blocks unrelated PRs | Set floor below measured `main`; bump only in 5.0.1 (or dedicated CI PR). |
| Codecov token missing on forks | Upload step `continue-on-error` / conditional on secret. |
| Model swap leaves orphan embeddings | Filter at query time; indexer continues upserting current model (existing PK upsert). |
| Rank “normalization” scope creep | Document semantics only in 5.0.4; no cross-backend score unification in this cluster. |
| Low-value coverage tests | Prefer error paths and trait impls that encode contracts, not line-padding. |

## Out of scope

- SQLite semantic search (`sqlite-vec` / extension maturity).
- Per-model embedding tables or mixed-dimension vectors (schema partition).
- Raising coverage to an arbitrary % target (e.g. 50%) — incremental uplift only.
- SSE for MCP `resources/subscribe` (Cluster B deferral).
- Postgres `LISTEN`/`NOTIFY` at-most-once semantics (standing risk).
- Re-embedding entire workspace on provider change (manual reindex / future cluster).

## Dependencies

- **5.0.1** and **5.0.2** are independent; either may merge first.
- **5.0.3** before **5.0.4** (docs reference the shipped fields and SQL filter).
- **5.0.2** may land before or after **5.0.1**; do not tie Codecov to the new floor.

## Alternative next cluster (not this wave)

**Cluster 6 — Delivery reliability** (`v6.0.0`): bus gap metrics, subscribe
session observability, indexer staleness defaults — if operational pain outweighs
search/coverage work.

## References

- Coverage gate: [[Retros/Cluster 3.0]] (#148), `.github/workflows/ci.yml`.
- Deferred from [[Retros/Cluster 4.0]] and [[Open Work]].
- Embeddings schema: `migrations/postgres/0003_embeddings.sql` (`model` column).
- Model swap test pattern: `maidan-search/tests/embeddings.rs`.
