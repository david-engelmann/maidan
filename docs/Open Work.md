# Open work

Aggregate of everything deferred across retros plus standing risks.
The "if I had two hours, what could I work on" backlog.

Updated at the close of each cluster or optional minor retro. Items move
from "open" to "shipped" when the owning release merges its retro PR.

## Standing risks (still open)

- **At-most-once delivery on the event bus.** Postgres
  `LISTEN`/`NOTIFY` is fire-and-forget. `maidan_events` + replay HTTP
  API shipped in Cluster D; WS/MCP auto-replay on lag shipped in **`v3.0.0`**
  when `filter.workspace_id` is set; reconnect uses signed `resume_token`
  (**`v4.0.0`**); `replay_truncated` signals when one replay window is insufficient.
- **Bootstrap flags are high-impact.** Bootstrap routes are now gated by
  `MAIDAN_BOOTSTRAP=1` when auth is enabled (`v1.4.1`), but leaving
  `AUTH_DISABLED` or bootstrap flags on outside controlled seed windows
  still creates avoidable exposure.
- **Indexer staleness is opt-in.** Set `INDEXER_STALE_SECS` to mark
  `/health/ready` degraded when the indexer has not observed an event
  recently. Default `0` disables the check.
- **PostgresBus listener recovery is best-effort.** `/health/ready` reports
  `bus: error` while the background task is in a retry loop (`v1.1.0`); it
  clears after the next successful `recv`.
- **Coverage depth is still low.** CI enforces a **10.0%** line floor
  (**`v5.0.0`**); optional Codecov upload when `CODECOV_TOKEN` is set.
  Further uplift (e.g. 11%+) needs a measured bump PR.
- **SQLite has no semantic search.** `Search::semantic_search`
  returns `Unsupported`. → `sqlite-vec` when mature.
- **`hash-v1` is not semantic.** Real provider support shipped in `v1.3.0`,
  but default deployments may still run deterministic `hash-v1` if not configured.

## Shipped post-1.0

| Release / area | Highlights |
|----------------|------------|
| Tracks T–X | See [[Post-1.0]] (closure #121) |
| **`v7.0.0`** | Postgres bus NOTIFY pointer + hydrate — [[Retros/Cluster 7.0]] |
| **`v6.0.0`** | Delivery reliability metrics + runbooks — [[Retros/Cluster 6.0]] |
| **`v5.0.0`** | Coverage floor 10%, Codecov optional, model-aware semantic search — [[Retros/Cluster 5.0]] |
| **`v1.4.0`** | Bootstrap one-shot gate + OIDC design spike — [[Retros/Minor 1.4]] |
| **`v1.2.0`** | Embedding provider hook, search facets, Postgres websearch operators — [[Retros/Minor 1.2]] |
| **`v1.1.0`** | Bus health, WS replay, federation secrets — [[Retros/Minor 1.1]] |

**Still manual:** Sigstore/cosign of release artifacts (V.3 — [[Operations]]).

## Recently closed: `v7.0.0`

Bus pointer delivery — [[Retros/Cluster 7.0]] (`Store::get_stored_event`, Postgres
`log_id_v1` NOTIFY + hydrate, large-event publish, docs).

Before that: `v6.0.0` delivery reliability — [[Retros/Cluster 6.0]].

Before that: `v5.0.0` coverage & search quality — [[Retros/Cluster 5.0]].

## Active plan: Cluster 8.0

Bus hydrate observability — [[Clusters/Cluster 8.0]] (`v8.0.0`):

- `maidan_bus_notify_hydrate_total{result}` counters (8.0.1)
- Production/Operations hydrate alerting and troubleshooting (8.0.2)
- Integration tests for hydrate failure paths (8.0.3)

## Still deferred (no owner yet)

| What | Notes |
|------|-------|
| Per-model embedding tables / mixed dimensions | 5.0 filters by model at query time; table split deferred |
| Resumable WS beyond `after_id` | Shipped **`v4.0.0`** — signed `resume_token` |
| S3 multipart for multi-GB blobs | Cluster E follow-up |
| OAuth/OIDC implementation | Shipped **`v2.0.0`** — [[Retros/Cluster 2.0]] |
| `MAIDAN_BOOTSTRAP=1` one-shot seed flag | Shipped **`v1.4.1`** (#129) |
| SSE for MCP `resources/subscribe` | Cluster B retro |
| Schema parity property test (`information_schema`) | Cluster A retro |
| Score normalization across Postgres vs SQLite ranks | Documented in 5.0; unification deferred |
| Coverage minimum gate | Shipped **`v3.0.0`**; floor raised **`v5.0.0`**; Codecov optional **`v5.0.0`** |
| SQLite file-backed durability tests | Cluster V retro |
| HorizontalPodAutoscaler manifest | Cluster A retro |
| Helm chart alternative to Kustomize | Cluster A plan |

## Known state at this handoff

- **Latest tag:** `v7.0.0` — Bus pointer delivery.
- **Active cluster:** **Cluster 8.0** — see [[Clusters/Cluster 8.0]].
- **Docs site:** mdBook on `main`; enable GitHub Pages in repo settings if not live.

## How to read this file

- The "Standing risks" list at the top is the always-on register.
- [[Post-1.0]] is the live minor-release ladder; this file is the backlog.
- A retro PR is the legitimate moment to add deferred items.
