# Cluster 1.0 retro — Production gates

> Closing wave for Cluster 1.0 · target tag `v1.0.0`.

Clusters A–H built the product surface. Cluster 1.0 declares the
**semver freeze**: production deployment guidance, config guards, and
health probe semantics are documented and load-bearing.

## What shipped

- **PR #92** (Cluster H) — `/health/live`, `/health/ready`, `docs/Production.md`, `MAIDAN_ENV=production` guard (landed ahead of 1.0 retro).
- **1.0.retro PR** — `v1.0.0` documentation + API stability decision.

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Track V   | Sigstore-signed artifacts                         | Security track.                          |
| Track W   | OpenAPI / docs site                               | Documentation track.                     |
| Post-1.0  | OAuth/OIDC human login                            | Agents use API tokens.                   |
| Post-1.0  | GDPR hard-delete flow                             | Cluster V.                               |

## Surprises

- **Probe split shipped in H** — 1.0 retro mostly documents and tags rather than new code.

## Decisions

- **API stability from `v1.0.0`** — HTTP routes and MCP tool shapes are semver-stable; breaking changes require `v2.0.0`.
- **`MAIDAN_ENV=production`** — cannot combine with `AUTH_DISABLED` (see `docs/Production.md`).

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Production runbook (`docs/Production.md`)               | `v1.0.0`           |
| Liveness/readiness probe contract                       | `v1.0.0` (impl in `v0.7.0`) |
| Config guard: production forbids `AUTH_DISABLED`        | `v1.0.0` (impl in `v0.7.0`) |
| Semver-stable public API                                | `v1.0.0`           |

## Risks identified + still open

- **Bootstrap routes** remain unauthenticated — production flow documented, not enforced in code.
- **Peer outbound secrets** lost on restart (Cluster G) — fixed in `v1.1.0`.

## Forward look

Cross-cutting tracks T–X (telemetry, perf, security, docs site, release engineering) continue without version tags.

## Acknowledgements

Solo cluster. Code gates largely landed in Cluster H; 1.0 closes the ladder.
