# Cluster 117.0 retro — Pluggable production provider

> Tag **`v117.0.0`**. Second cluster of **Phase XXII (Search & indexer at scale)**.

## What shipped

- **Dimension auto-detect** (117.0.1): the `openai-compatible` provider's
  `MAIDAN_EMBEDDING_DIM` previously defaulted to `1024` (hash-v1's dimension),
  which silently mismatched every real model and failed each embed. Now, when
  the env var is unset, the provider issues one sentinel embed at construction
  and uses the returned length — so a wrong model id or unreachable endpoint
  fails **at boot with a clear error** rather than per-message. An explicit
  dimension skips the probe; a configured `0` is rejected. The POST/parse path
  is factored into `request_embeddings` (shared by `embed` / `embed_batch` /
  the probe) and `resolve_dimension` is a unit-tested pure helper.
- **Startup model registration** (117.0.2): new `Search::ensure_model(provider)`
  (default no-op; Postgres/SQLite create the per-model table + registry row),
  called once at server boot. A freshly-configured model is queryable before
  the first write, and a `DimensionMismatch` surfaces in startup logs. The call
  is **non-fatal** — a registration hiccup logs a warning and the per-message
  write path retries lazily.
- **Migration/reindex docs** (117.0.3): `docs/Embeddings.md` covers provider
  selection, the `openai-compatible` env surface (including the auto-detect
  probe), the per-model table scheme, startup registration, and the
  switch-models / reindex workflow (HTTP `POST /operator/reindex-embeddings`
  and `maidan-cli reindex-embeddings`, HNSW rebuild caveat, rollback). Linked
  from the docs index and the mdBook SUMMARY.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster 118  | Hybrid lexical+semantic relevance | Next on the ladder. |
| (future)     | Per-request retry/backoff in the provider | The timeout is configurable; retries are a separable resilience feature. |
| (future)     | Async-native remote client | `spawn_blocking` (Cluster 116) keeps the blocking client off the runtime; a native async client is a larger refactor. |
| (future)     | Make startup registration boot-fatal on `DimensionMismatch` | Kept non-fatal so messaging always boots; revisit if operators want fail-fast. |

## Surprises

- **Migrations seed `hash-v1`.** The first `ensure_model` test asserted the
  model wasn't registered until `ensure_model` ran — but `run_sqlite_migrations`
  pre-seeds the `hash-v1` registry row, so the assertion failed. Rewrote the
  test to register a *new* model id via a tiny `FakeProvider`, which is also a
  truer test of the migration scenario (switch to a model the registry hasn't
  seen).
- **The per-model scheme was already most of the way there.** `upsert_embedding`
  already lazily `ensure_model`s with the *actual* embedding length, so the
  scheme self-heals on first write. 117's value was (a) removing the dimension
  footgun so the provider works at all, and (b) registering at boot so misconfig
  surfaces early rather than on the first message.

## Decisions

- **Probe to auto-detect dimension, fail-fast at boot.** Operators shouldn't
  have to know a model's output dimension; probing once at construction both
  removes the footgun and validates connectivity at boot. Explicit
  `MAIDAN_EMBEDDING_DIM` opts out (air-gapped boot, or asserting an expected
  dimension). No [[Decisions]] change.
- **Startup registration is non-fatal.** A chat server must boot and serve
  messaging even if the embedding subsystem is misconfigured. The error is
  logged loudly and the write path retries; embeddings are best-effort.
- **`ensure_model` as a trait method, not a free function in `main`.** Keeps the
  pool/HNSW params encapsulated in each backend (mirrors `reindex_embeddings`)
  and gives `main` a one-liner.

## Capability table extension

| Capability | Where |
|------------|-------|
| Production `openai-compatible` embeddings with auto-detected dimension | `crates/maidan-search/src/embedding_provider.rs` |
| Boot-time per-model registration (`Search::ensure_model`) | `crates/maidan-search/src/traits.rs`, `postgres.rs`, `sqlite.rs`; called in `maidan-server/src/main.rs` |
| Embedding provider + model-migration guide | `docs/Embeddings.md` |

## Risks identified + mitigated

- **Silent per-message embed failure on dimension mismatch.** Replaced by a
  boot-time probe/registration that surfaces the error immediately.

## Risks identified + still open

- **Boot probe couples startup to endpoint availability** (only when
  `MAIDAN_EMBEDDING_DIM` is unset). Documented; set the dimension explicitly to
  decouple. A transient endpoint blip at boot fails provider construction —
  acceptable fail-fast for a misconfigured embedding endpoint.

## Forward look

Phase **XXII** concludes with **Cluster 118 — hybrid relevance**: optional
hybrid lexical+semantic ranking over the normalized `[0,1]` score, with a small
relevance-eval harness guarding regressions.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
