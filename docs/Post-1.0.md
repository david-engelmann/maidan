# Post-1.0 delivery

The versioned cluster ladder (A–H + **1.0**) is complete at tag
`v1.0.0`. Further work ships as **cross-cutting tracks** (no mandatory
retro tag) or as optional **minor releases** (`v1.1.0`, …) when a
coherent capability batch warrants semver + changelog.

## Current focus

| Track | Status | Plan |
|-------|--------|------|
| **T** | In progress | [[Tracks/Track T]] — OTLP + indexer health shipped; T.3 coverage optional. |
| **U** | Not started | [[Tracks/Track U]] — benches, mutation tests, query tuning. |
| **V** | Not started | [[Tracks/Track V]] — threat model, GDPR erasure, signed releases. |
| **W** | In progress (W.1) | [[Tracks/Track W]] — OpenAPI, docs site. |
| **X** | Not started | [[Tracks/Track X]] — release automation hygiene. |

**Recommended order:** finish **Track T** → **Track W** (API reference for
stable `v1.0.0` surface) → **Track V** (production hardening) → **Track U**
(perf regression gates) → **Track X**.

## Optional minor: `v1.1.0` — delivery reliability ✓

Shipped at tag **`v1.1.0`** ([[Retros/Minor 1.1]]). Addresses top standing risks
in [[Open Work]] without breaking the public API.

| PR   | Scope | Outcome |
|------|-------|---------|
| 1.1.1 ✓ | PostgresBus listener health on `/health/ready` | Operator sees bus degradation. |
| 1.1.2 ✓ | WS subscriber gap detection + replay hint | Server tells clients `after_id` to poll when NOTIFY missed. |
| 1.1.3 ✓ | Resumable WS: `after_id` on subscribe | Clients reconnect without full replay from 0. |
| 1.1.4 ✓ | Federation: persist peer outbound secrets (migration) | Pull worker survives restart. |
| 1.1.5 ✓ | Federation pull compose smoke | Two-instance pull path CI. |
| 1.1.retro ✓ | Capabilities + changelog + tag `v1.1.0` | Same close pattern as clusters. |

**Not in 1.1:** OAuth/OIDC, real ML embeddings, GDPR hard-delete (Track V /
post-1.0 product).

## Optional minor: `v1.2.0` — search + embeddings

| PR   | Scope |
|------|-------|
| 1.2.1 | Pluggable embedding provider trait + config (keep `hash-v1` default). |
| 1.2.2 | Faceted search filters on `GET …/search` (author, channel, kind). |
| 1.2.3 | `websearch_to_tsquery` operator pass-through (Postgres). |
| 1.2.retro | Tag `v1.2.0`. |

## Track T — remaining (no tag)

| PR   | Title | Notes |
|------|-------|-------|
| T.3  | `chore(ci): cargo-llvm-cov` + optional Codecov upload | Closes “no coverage gate” risk. |
| T.4  | Prometheus `/metrics` exporter | Deferred from Track T out-of-scope. |
| T.5  | PostgresBus supervision + readiness | Overlaps 1.1.1 if not done earlier. |
| T.6  | SQLite WAL + `busy_timeout` PRAGMAs | Dev ergonomics. |

## Track W — API docs (high leverage post-1.0)

| PR   | Title |
|------|-------|
| W.1  | `utoipa` OpenAPI from axum routes + problem+json schemas |
| W.2  | mdBook site publishing `docs/` + generated OpenAPI |
| W.3  | MCP tool catalog page synced from `maidan-mcp` |

## Track V — security

| PR   | Title |
|------|-------|
| V.1  | Threat model doc + bootstrap route hardening options |
| V.2  | GDPR erasure flow (tombstone → purge) |
| V.3  | Sigstore cosign on release artifacts (pairs with Track X) |

## Issue filing

Before coding **1.1.x**, open a GitHub milestone `v1.1.0` and one issue
per PR row above (label `cluster:1.1` or `track:reliability`). Tracks T–X
use labels `track:T`, etc.

## Decision: clusters vs tracks

- **Tracks** — infra/ops/docs/perf; ship PR-by-pr on `main`.
- **Minors (`v1.1.0`)** — user-visible reliability or search upgrades;
  require retro PR + tag like clusters, but semver stays `1.x` until a
  breaking API change forces `v2.0.0`.
