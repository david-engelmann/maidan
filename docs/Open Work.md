# Open work

Aggregate of everything deferred across retros plus standing risks.
The "if I had two hours, what could I work on" backlog.

Updated at the close of each cluster or optional minor retro. Items move
from "open" to "shipped" when the owning release merges its retro PR.

## Standing risks (still open)

- **At-most-once delivery on the event bus.** Postgres
  `LISTEN`/`NOTIFY` is fire-and-forget. `maidan_events` + replay HTTP
  API shipped in Cluster D, but subscribers must poll replay on gap —
  no automatic WS backfill on lag beyond `replay_hint` (v1.1.2); subscribe
  `after_id` replays from `maidan_events` on connect (v1.1.3).
- **Bootstrap routes are unauthenticated.** `POST /workspaces` and
  `POST …/members` have no Bearer gate; production must seed with
  `AUTH_DISABLED` then mint tokens before enabling auth.
- **Indexer staleness is opt-in.** Set `INDEXER_STALE_SECS` to mark
  `/health/ready` degraded when the indexer has not observed an event
  recently. Default `0` disables the check.
- **PostgresBus listener recovery is best-effort.** `/health/ready` reports
  `bus: error` while the background task is in a retry loop (`v1.1.0`); it
  clears after the next successful `recv`.
- **No coverage threshold in CI.** `cargo-llvm-cov` uploads `lcov.info` as a
  CI artifact (Track T.3); no minimum % gate or Codecov upload yet.
- **SQLite has no semantic search.** `Search::semantic_search`
  returns `Unsupported`. → post-1.0 candidate via `sqlite-vec` if the
  extension's sqlx integration matures.
- **`hash-v1` is not semantic.** Pluggable provider trait shipped in
  `v1.2.1`; real ML models still need an external provider implementation.

## Shipped post-1.0 (tracks T, U, V, W, X)

See [[Post-1.0]] (closure PR #121). Highlights:

| Area | Shipped |
|------|---------|
| T | OTLP, indexer on `/health/ready`, llvm-cov artifact, Prometheus `/metrics`, SQLite WAL + `busy_timeout` |
| U | `criterion` store bench, nightly `cargo-mutants`, [[Query-Tuning]], WS 100-event soak |
| V | [[Threat-Model]], `DELETE /messages/:id/purge`, k8s `NetworkPolicy` |
| W | `GET /openapi.json`, mdBook + GitHub Pages, MCP reference generation |
| X | CycloneDX SBOM on release workflow, prod digest docs |

**Still manual:** Sigstore/cosign of release artifacts (V.3 — documented in [[Operations]]).

## Deferred to optional `v1.2.0` minor

| What | PR |
|------|-----|
| Pluggable embedding provider + `MAIDAN_EMBEDDING_PROVIDER` | 1.2.1 ✓ |
| Faceted search (author / channel / kind on `GET …/search`) | 1.2.2 (in flight) |
| `websearch_to_tsquery` operator pass-through (Postgres `q`) | 1.2.3 |

## Still deferred (no owner yet)

| What | Notes |
|------|-------|
| Real ML embedding model | Implement `EmbeddingProvider` + wire env |
| Resumable WS beyond `after_id` | Reconnection tokens, automatic NOTIFY replay |
| Per-model embedding tables / mixed dimensions | Schema + search API |
| S3 multipart for multi-GB blobs | Cluster E follow-up |
| OAuth/OIDC | Auth track / product |
| MCP stdio transport | Cluster H retro |
| SSE for MCP `resources/subscribe` | Cluster B retro |
| Schema parity property test (`information_schema`) | Cluster A retro |
| Score normalization across Postgres vs SQLite ranks | Cluster C retro |
| Coverage upload site / Codecov + minimum gate | Track W partial |
| `MAIDAN_BOOTSTRAP=1` one-shot seed flag | Threat model T1 — defer to v2 |
| SQLite file-backed durability tests | Cluster V retro |
| HorizontalPodAutoscaler manifest | Cluster A retro |
| Helm chart alternative to Kustomize | Cluster A plan |

## Known state at this handoff

- **Latest tag:** `v1.1.0` (optional minor — delivery reliability). Post-1.0
  tracks T/U/V/W/X closed at `main` after #121.
- **Active optional minor:** **`v1.2.0`** — search + embeddings ladder in
  [[Post-1.0]]; PR **1.2.1** next.
- **Docs site:** mdBook workflow ships on `main`; enable GitHub Pages in repo
  settings if the site is not live yet.

## How to read this file

- The "Standing risks" list at the top is the always-on register.
  Items leave the list when the underlying issue is fixed.
- Shipped tables are historical pointers; [[Post-1.0]] is the live plan.
- A retro PR is the legitimate moment to add deferred items. If you spot
  a gap, open a follow-up PR that updates this file.
