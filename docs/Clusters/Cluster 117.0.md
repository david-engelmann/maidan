# Cluster 117.0 — Pluggable production provider

**Theme:** Make the `openai-compatible` embedding provider first-class — usable in production without footguns — and document switching models.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXII · tag **`v117.0.0`**.

**Predecessor:** the `openai-compatible` provider (env-configured) and the per-model table scheme (v47); batch embedding (116).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Provider** | Auto-detect embedding dimension via a boot-time probe when `MAIDAN_EMBEDDING_DIM` is unset (fail-fast on misconfig); explicit dim opts out. |
| **Registry** | `Search::ensure_model(provider)` — register the active model's table + registry row at boot; non-fatal. |
| **Docs** | `docs/Embeddings.md`: provider config, per-model scheme, switch-models/reindex workflow. |

## Non-goals

- Provider retry/backoff — timeout is configurable; retries are separable.
- Async-native client — `spawn_blocking` (116) suffices.
- Boot-fatal registration — kept non-fatal so messaging always boots.

## PR ladder (actual)

| # | Title |
|---|--------|
| 117.0.1 | `feat(search): auto-detect embedding dimension for openai-compatible provider` (#321) |
| 117.0.2 | `feat(search): register active embedding model at startup` (#321) |
| 117.0.3 | `docs(embeddings): provider config + model migration/reindex story` (#321) |
| 117.0.retro | `docs(retro): Cluster 117.0 + v117.0.0 tag prep` |

## Exit criteria

- First-class `openai-compatible` path with tunable dimension/model — **met** (auto-detect or explicit).
- Slots into the per-model table scheme (v47) — **met** (`ensure_model` at boot; models coexist; dim pinned per model).
- Migration/reindex story documented — **met** (`docs/Embeddings.md`).
- `v117.0.0` tagged after retro.

## Ordering & risks

- **Provider fix first (117.0.1):** without correct dimension the provider is unusable; everything else builds on it.
- **Risk — boot coupling:** the probe couples startup to endpoint availability when dim is unset; mitigated by documenting the explicit-dim opt-out.
- **Risk — hot-path wiring:** `ensure_model` at boot + the provider refactor; covered by search lib + `embedding_indexer`/`embedding_models` tests.

## References

- [[Clusters/Product Ladder 102+]] Phase XXII
- [[Retros/Cluster 117.0]], [Embeddings.md](../Embeddings.md)
