# Post-1.0 delivery

The versioned cluster ladder (A–H + **1.0**) is complete at tag
`v1.0.0`. Further work ships as **cross-cutting tracks** (no mandatory
retro tag) or as optional **minor releases** (`v1.1.0`, …) when a
coherent capability batch warrants semver + changelog.

## Current focus

| Track | Status | Plan |
|-------|--------|------|
| **T** | Complete | [[Tracks/Track T]] — OTLP, indexer health, coverage, Prometheus, SQLite WAL. T.5 bus health shipped in `v1.1.0`. |
| **U** | Complete | [[Tracks/Track U]] — criterion bench, nightly mutants, EXPLAIN playbook, WS soak. |
| **V** | Complete | [[Tracks/Track V]] — threat model, message purge, NetworkPolicy. V.3 cosign documented as manual (see Operations). |
| **W** | Complete | [[Tracks/Track W]] — OpenAPI, mdBook, MCP reference. |
| **X** | Complete | [[Tracks/Track X]] — release SBOM, prod digest docs, nightly/release hygiene. |

**Recommended order:** tracks and **`v1.2.0`** are closed. Next optional
minor is **`v1.3.0`** (real embeddings + semantic search UX) — see ladder
below — or ad-hoc items in [[Open Work]].

## Optional minor: `v1.2.0` — search + embeddings ✓

Shipped at tag **`v1.2.0`** ([[Retros/Minor 1.2]]).

| PR   | Scope |
|------|-------|
| 1.2.1 ✓ | Pluggable embedding provider + `MAIDAN_EMBEDDING_PROVIDER` (#122). |
| 1.2.2 ✓ | Faceted lexical search (`author`, `channel`, `kind`) (#123). |
| 1.2.3 ✓ | Postgres `websearch_to_tsquery` pass-through (#124). |
| 1.2.retro ✓ | Capabilities + changelog + tag `v1.2.0`. |

## Optional minor: `v1.3.0` — semantic search UX (proposed)

Natural follow-on to 1.2: expose semantic search and ship a real embedding
provider without breaking the lexical API.

| PR   | Scope |
|------|-------|
| 1.3.1 | `GET …/search?mode=semantic` (Postgres) or dedicated route; document rank semantics. |
| 1.3.2 | Remote embedding provider (OpenAI-compatible HTTP) + config. |
| 1.3.3 | Indexer failure visibility on `/health/ready` when embeddings enabled. |
| 1.3.retro | Tag `v1.3.0`. |

**Not in 1.3:** OAuth/OIDC, coverage % gate, SQLite semantic vectors.

## Optional minor: `v1.4.0` — auth hardening (proposed)

| PR   | Scope |
|------|-------|
| 1.4.1 | `MAIDAN_BOOTSTRAP=1` one-shot workspace seed gate. |
| 1.4.2 | OIDC login spike / design doc (or defer to `v2.0.0`). |
| 1.4.retro | Tag `v1.4.0`. |

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

## Track T — shipped

| PR   | Title | Notes |
|------|-------|-------|
| T.1–T.2 ✓ | OTLP + indexer on `/health/ready` | Shipped pre–post-1.0. |
| T.3 ✓ | `cargo-llvm-cov` coverage job | #113. |
| T.4 ✓ | Prometheus `/metrics` | #118. |
| T.5 ✓ | PostgresBus readiness | `v1.1.1` `bus_listener_health`. |
| T.6 ✓ | SQLite WAL + `busy_timeout` | #117. |

## Track W — shipped

| PR   | Title |
|------|-------|
| W.1 ✓ | OpenAPI via utoipa (#114) |
| W.2 ✓ | mdBook + GitHub Pages (#115) |
| W.3 ✓ | MCP reference generator (#116) |

## Track V — shipped

| PR   | Title |
|------|-------|
| V.1 ✓ | Threat model + bootstrap notes (#119) |
| V.2 ✓ | `DELETE /messages/:id/purge` (#120) |
| V.3  | Cosign on release binaries — manual step in [[Operations]] until keyless CI is configured |
| V.4 ✓ | `NetworkPolicy` in `k8s/base` |

## Track U — shipped

| PR   | Title |
|------|-------|
| U.1 ✓ | `maidan-store` criterion bench `store_hot` |
| U.2 ✓ | Nightly `cargo-mutants` (`.github/workflows/nightly.yml`) |
| U.3 ✓ | [[Query-Tuning]] playbook (#119) |
| U.4 ✓ | 100-event WS soak test |

## Track X — shipped

| PR   | Title |
|------|-------|
| X.1 ✓ | Release workflow + compose smoke on every PR |
| X.2 ✓ | Prod digest pinning documented in `k8s/README.md` |
| X.3 ✓ | `cargo-cyclonedx` SBOM on release |

## Issue filing

Before coding **1.1.x**, open a GitHub milestone `v1.1.0` and one issue
per PR row above (label `cluster:1.1` or `track:reliability`). Tracks T–X
use labels `track:T`, etc.

## Decision: clusters vs tracks

- **Tracks** — infra/ops/docs/perf; ship PR-by-pr on `main`.
- **Minors (`v1.1.0`)** — user-visible reliability or search upgrades;
  require retro PR + tag like clusters, but semver stays `1.x` until a
  breaking API change forces `v2.0.0`.
