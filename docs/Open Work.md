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
- **Coverage depth is still low.** CI enforces a line floor (`COVERAGE_MIN_LINES`,
  shipped **`v3.0.0`**); meaningful uplift and Codecov upload are in flight
  (**Cluster 5.0**).
- **SQLite has no semantic search.** `Search::semantic_search`
  returns `Unsupported`. → `v1.3.0+` or `sqlite-vec` when mature.
- **`hash-v1` is not semantic.** Real provider support shipped in `v1.3.0`,
  but default deployments may still run deterministic `hash-v1` if not configured.

## Shipped post-1.0

| Release / area | Highlights |
|----------------|------------|
| Tracks T–X | See [[Post-1.0]] (closure #121) |
| **`v1.4.0`** | Bootstrap one-shot gate + OIDC design spike — [[Retros/Minor 1.4]] |
| **`v1.2.0`** | Embedding provider hook, search facets, Postgres websearch operators — [[Retros/Minor 1.2]] |
| **`v1.1.0`** | Bus health, WS replay, federation secrets — [[Retros/Minor 1.1]] |

**Still manual:** Sigstore/cosign of release artifacts (V.3 — [[Operations]]).

## Recently closed: `v4.0.0`

Subscriber continuity — [[Retros/Cluster 4.0]] (resume tokens, `replay_truncated`, subscribe docs).

Before that: `v3.0.0` search & subscriber depth — [[Retros/Cluster 3.0]].

## Active plan: Cluster 5.0

Coverage & search quality — [[Clusters/Cluster 5.0]] (`v5.0.0`):

- Targeted tests + raised `COVERAGE_MIN_LINES` (5.0.1)
- Optional Codecov upload from CI (5.0.2)
- Semantic search filtered by active embedding `model` + hit metadata (5.0.3)
- Rank semantics documented in Architecture / OpenAPI (5.0.4)

## Still deferred (no owner yet)

| What | Notes |
|------|-------|
| Per-model embedding tables / mixed dimensions | Cluster 5.0 filters by model; table split deferred |
| Resumable WS beyond `after_id` | Shipped **`v4.0.0`** — signed `resume_token` |
| S3 multipart for multi-GB blobs | Cluster E follow-up |
| OAuth/OIDC implementation | Shipped **`v2.0.0`** — [[Retros/Cluster 2.0]] |
| `MAIDAN_BOOTSTRAP=1` one-shot seed flag | Shipped **`v1.4.1`** (#129) |
| SSE for MCP `resources/subscribe` | Cluster B retro |
| Schema parity property test (`information_schema`) | Cluster A retro |
| Score normalization across Postgres vs SQLite ranks | Cluster C retro |
| Coverage minimum gate | Shipped in **`v3.0.0`**; Codecov upload still deferred |
| SQLite file-backed durability tests | Cluster V retro |
| HorizontalPodAutoscaler manifest | Cluster A retro |
| Helm chart alternative to Kustomize | Cluster A plan |

## Known state at this handoff

- **Latest tag:** `v4.0.0` — Subscriber continuity.
- **Active cluster:** **Cluster 5.0** — see [[Clusters/Cluster 5.0]].
- **Docs site:** mdBook on `main`; enable GitHub Pages in repo settings if not live.

## How to read this file

- The "Standing risks" list at the top is the always-on register.
- [[Post-1.0]] is the live minor-release ladder; this file is the backlog.
- A retro PR is the legitimate moment to add deferred items.
