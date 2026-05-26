# Open work

Aggregate of everything deferred across retros plus standing risks.
The "if I had two hours, what could I work on" backlog.

Updated at the close of each cluster or optional minor retro. Items move
from "open" to "shipped" when the owning release merges its retro PR.

## Standing risks (still open)

- **At-most-once delivery on the event bus.** Postgres
  `LISTEN`/`NOTIFY` is fire-and-forget. `maidan_events` + replay HTTP
  API shipped in Cluster D; subscribers must poll replay on gap —
  no automatic WS backfill on lag beyond `replay_hint` (v1.1.2); subscribe
  `after_id` replays from `maidan_events` on connect (v1.1.3).
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
- **No coverage threshold in CI.** `cargo-llvm-cov` uploads `lcov.info` as a
  CI artifact (Track T.3); no minimum % gate or Codecov upload yet.
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

## Recently closed: `v2.1.0`

OIDC operator hardening — [[Retros/Cluster 2.1]] (signed cookies, IdP logout,
OpenAPI auth routes, optional auto-mint).

Before that: runtime OIDC + human sessions at **`v2.0.0`** — [[Retros/Cluster 2.0]].

## Active plan: Cluster 3.0

Search facets, coverage CI gate, WS gap auto-replay — sketched in
[[Clusters/Cluster 2.1]]; dedicated cluster plan TBD.

## Still deferred (no owner yet)

| What | Notes |
|------|-------|
| Semantic search facets | After `v1.3.0` baseline; needs rank/filter semantics |
| Per-model embedding tables / mixed dimensions | Schema + search API |
| Resumable WS beyond `after_id` | Reconnection tokens, automatic NOTIFY replay |
| S3 multipart for multi-GB blobs | Cluster E follow-up |
| OAuth/OIDC implementation | Shipped **`v2.0.0`** — [[Retros/Cluster 2.0]] |
| `MAIDAN_BOOTSTRAP=1` one-shot seed flag | Shipped **`v1.4.1`** (#129) |
| SSE for MCP `resources/subscribe` | Cluster B retro |
| Schema parity property test (`information_schema`) | Cluster A retro |
| Score normalization across Postgres vs SQLite ranks | Cluster C retro |
| Coverage upload site / Codecov + minimum gate | Track T partial |
| SQLite file-backed durability tests | Cluster V retro |
| HorizontalPodAutoscaler manifest | Cluster A retro |
| Helm chart alternative to Kustomize | Cluster A plan |

## Known state at this handoff

- **Latest tag (after retro merges):** `v2.1.0` — OIDC operator hardening.
- **Recommended next:** kick off **Cluster 3.0** or tackle standing risks below.
- **Docs site:** mdBook on `main`; enable GitHub Pages in repo settings if not live.

## How to read this file

- The "Standing risks" list at the top is the always-on register.
- [[Post-1.0]] is the live minor-release ladder; this file is the backlog.
- A retro PR is the legitimate moment to add deferred items.
