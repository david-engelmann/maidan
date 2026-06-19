# Cluster 118.0 — Hybrid relevance

**Theme:** Fuse lexical + semantic search into an optional hybrid ranking over the normalized `[0,1]` score, and guard ranking quality with a relevance eval harness.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXII · tag **`v118.0.0`** (closes the phase).

**Predecessor:** normalized lexical + semantic scores (`score.rs`); the batch pipeline (116) and pluggable provider (117).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Fusion** | `score::fuse_hybrid` — weighted sum of normalized `[0,1]` scores; `Search::hybrid_search` default method composing the two surfaces. |
| **API** | `SearchMode::Hybrid` + `hybrid_weight` on HTTP search and the MCP `search_messages` tool. |
| **Eval** | `tests/relevance_eval.rs` — controlled synonym embedding + labeled corpus; recall@k / MRR regression guards. |

## Non-goals

- Reciprocal-rank fusion (the ladder specified normalized-score fusion).
- Learned / per-workspace weights — default 0.5 + per-request override.
- New SQL or per-backend hybrid implementations.

## PR ladder (actual)

| # | Title |
|---|--------|
| 118.0.1 | `feat(search): hybrid lexical+semantic ranking` (#323) |
| 118.0.2 | `test(search): relevance eval harness guarding hybrid ranking` (#323) |
| 118.0.retro | `docs(retro): Cluster 118.0 + v118.0.0 tag prep` |

## Exit criteria

- Optional hybrid ranking over the normalized `[0,1]` score — **met**.
- A small relevance eval harness guards regressions — **met**.
- `v118.0.0` tagged after retro (closes Phase XXII).

## Ordering & risks

- **Fusion first (118.0.1):** the eval harness (118.0.2) exercises it.
- **Risk — eval fragility:** a toy embedding can't cleanly prove hybrid > semantic; the harness asserts structural recall dominance + lexical-recovery + an MRR floor (robust) rather than a contrived semantic miss.

## References

- [[Clusters/Product Ladder 102+]] Phase XXII
- [[Retros/Cluster 118.0]]
