# Cluster 118.0 retro — Hybrid relevance

> Tag **`v118.0.0`**. Third and final cluster of **Phase XXII (Search & indexer at scale)** — closes the phase.

## What shipped

- **Hybrid search mode** (118.0.1): `SearchMode::Hybrid` on both the HTTP search
  endpoint and the MCP `search_messages` tool. It runs lexical and semantic
  search and fuses their normalized `[0,1]` scores via `score::fuse_hybrid`:
  `combined = w*semantic + (1-w)*lexical`, where `w` is `hybrid_weight`
  (default `DEFAULT_HYBRID_WEIGHT = 0.5`, clamped to `[0,1]`). A result present
  in only one side contributes `0` on the other; ties break by `posted_at`
  descending. The lexical hit is the representative (it carries the FTS
  snippet) and is tagged with the semantic model when it also matched
  semantically. Implemented as a **default `Search::hybrid_search` trait
  method** composing the existing `search_messages` + `semantic_search`, so
  both backends inherit it with no per-backend SQL.
- **Relevance eval harness** (118.0.2): `tests/relevance_eval.rs` — a labeled
  corpus plus a controlled `SynonymProvider` (L2-normalized bag-of-concepts
  with synonym folding) so lexical/semantic/hybrid rankings are deterministic
  without a live model. It computes recall@k + reciprocal rank per mode and
  asserts the regression-guarding properties: hybrid recall dominates both
  single modes per query, hybrid's top hit is relevant (MRR ~1.0 floor), hybrid
  strictly beats lexical in aggregate (recovers synonym docs FTS misses), and a
  focused test that lexical `"car"` misses the `"automobile"` doc while hybrid
  recovers it.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Phase XXIII (119) | Dependency dedupe & currency | Phase XXII is closed; the ladder moves to supply-chain. |
| (future)     | Reciprocal-rank fusion (RRF) as an alternative to weighted-sum | The ladder specified fusion "over the normalized `[0,1]` score"; RRF is rank-based and a separable option. |
| (future)     | Per-workspace / learned `hybrid_weight` | Default 0.5 + per-request override is enough; tuning is a later concern. |
| (future)     | Batched query embedding for hybrid | Hybrid embeds the query once (single `embed`); no batching needed. |

## Surprises

- **A toy embedding can't cleanly show hybrid beating *semantic*.** With a
  bag-of-concepts embedding, semantic recall is high whenever the concept is
  captured, so the honest, robust demonstration is hybrid recovering *lexical's*
  synonym misses. Hybrid-vs-semantic is asserted as **structural** recall
  dominance (hybrid is a re-ranked union of both result sets, so with `k`
  large enough it can't recall fewer) plus a ranking-quality floor — rather
  than over-claiming with a contrived semantic miss. Recorded in the harness
  comments so the next maintainer doesn't "fix" it by forcing a fragile case.
- **The fusion needed no new query plumbing.** Because both backends already
  expose `search_messages` + `semantic_search` with normalized scores, hybrid
  is a pure default trait method + a pure fusion function — zero new SQL, and it
  works identically on Postgres and SQLite.

## Decisions

- **Weighted sum over normalized scores**, per the ladder, with the semantic
  weight tunable per request (`hybrid_weight`, default 0.5). Simple, explainable,
  and backend-agnostic. No [[Decisions]] change.
- **Default trait method, not per-backend.** `hybrid_search` composes existing
  methods, so there's one implementation and both backends inherit it.
- **Lexical hit as the representative** in fused results (keeps the FTS
  `<mark>` snippet), tagged with the semantic model when it also matched — best
  of both for display + observability.
- **Honest eval claims.** Assert what the harness can prove robustly (lexical
  recovery + structural dominance + MRR floor); document the rest.

## Capability table extension

| Capability | Where |
|------------|-------|
| Hybrid lexical+semantic search (HTTP + MCP) | `crates/maidan-server/src/routes/search.rs`, `crates/maidan-mcp/src/tools/search.rs` |
| Score fusion (`fuse_hybrid`, `DEFAULT_HYBRID_WEIGHT`) | `crates/maidan-search/src/score.rs`, `traits.rs` |
| Relevance eval harness | `crates/maidan-search/tests/relevance_eval.rs` |

## Risks identified + mitigated

- **Fusion ranking regressions.** The eval harness fails if hybrid stops
  dominating single-mode recall or its top hit stops being relevant.

## Risks identified + still open

- **Hybrid can surface a non-relevant semantic neighbor above a relevant
  lexical hit** (precision cost of recall). The harness guards top-1 relevance
  on the fixture; production relevance with real embeddings is a tuning concern
  (`hybrid_weight`), not a correctness one.

## Forward look

**Phase XXII (Search & indexer at scale) is complete** (116 batch pipeline →
117 pluggable provider → 118 hybrid relevance). The ladder moves to **Phase
XXIII — Supply chain & scale gate (Clusters 119–120)**, opening with **Cluster
119 — dependency dedupe & currency** (collapse duplicate majors, tighten
`deny.toml`, track `openidconnect` v5, evaluate edition 2024).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
