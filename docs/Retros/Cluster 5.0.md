# Cluster 5.0 retro — Coverage & search quality

> Closing wave for Cluster 5.0 · target tag `v5.0.0`.

Cluster 5.0 raised the CI coverage floor with targeted tests, optional Codecov
upload, model-filtered semantic search, and operator docs for rank semantics.

## What shipped

- **PR #155** — Cluster kickoff plan ([[Clusters/Cluster 5.0]]).
- **PR #156** — Implementation bundle (5.0.1–5.0.4):
  - `COVERAGE_MIN_LINES` **9.0 → 10.0**; unit tests for `filters::is_empty`,
    subscribe resume, listener health.
  - Codecov upload when `CODECOV_TOKEN` is set (`fail_ci_if_error: false`).
  - Postgres `semantic_search` filters on active embedding `model`;
    `SearchHit.embedding_model`; `/health` embedding metadata.
  - Architecture / Production rank semantics; OpenAPI `SearchHit` schema.

## What was deferred

| To           | What                                              | Why                                      |
|--------------|---------------------------------------------------|------------------------------------------|
| Cluster 6+   | Coverage floor toward 11%+ with measured uplift   | `main` ~10%; 11.0 gate failed first CI.  |
| Cluster 6+   | Per-model embedding tables / SQLite semantic      | Out of scope for 5.0.                    |
| Post-5.0     | Cross-backend rank normalization                  | Document-only in 5.0.4.                  |
| Cluster B    | SSE for MCP `resources/subscribe`                 | Long-standing deferral.                  |

## Surprises

- **`COVERAGE_MIN_LINES=11.0` failed** on first green measurement; floor set to
  **10.0** (run `26492169902`) per bump-below-measured policy.
- **Resume-token tamper test was flaky** — replacing MAC hex with `'a'` no-ops when
  the digit was already `a`; fixed by pop/push like session cookies.
- **Implementation shipped as one PR** — ladder items 5.0.1–5.0.4 squashed in #156
  for solo velocity; retro documents the split.

## Decisions

- **Model filter at query time** — `e.model = $provider.model_name()`; orphan rows
  from prior providers stay in DB but are invisible to semantic search.
- **`embedding_model` on hits** — populated for semantic Postgres hits only;
  lexical/SQLite omit the field.
- **Codecov optional** — upload gated on secret; never blocks fork PRs or local CI.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Raised CI line-coverage floor (10.0%)                   | `v5.0.0`           |
| Optional Codecov upload from `llvm-cov` artifact        | `v5.0.0`           |
| Model-aware Postgres semantic search                    | `v5.0.0`           |
| `SearchHit.embedding_model` + health embedding metadata | `v5.0.0`           |
| Documented lexical vs semantic rank semantics           | `v5.0.0`           |

## Risks identified + mitigated

- **Stale vectors after model swap** — query-time `model` filter; upsert still
  replaces rows for the active model via existing PK.
- **Coverage regression** — floor raised from 9.0 to 10.0 with targeted tests.

## Risks identified + still open

- **Coverage depth** — ~10% lines; floor prevents regression, not depth.
- **SQLite semantic search** — still unsupported.
- **At-most-once bus** — unchanged.

## Forward look

**Cluster 6** (candidate): delivery reliability — bus gap metrics, subscribe
observability, indexer staleness defaults — or incremental coverage toward 11%+.
See [[Open Work]] and [[Clusters/Cluster 5.0#Alternative next cluster]].

## Acknowledgements

Solo cluster. Kickoff #155, implementation #156, this retro.
