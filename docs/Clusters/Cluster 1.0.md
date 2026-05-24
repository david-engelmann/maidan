# Cluster 1.0 — Production gates

Cluster H delivered operator UI and transport polish. Cluster 1.0 is the
**semver freeze**: document production requirements, split health probes,
and lock the pre-1.0 "break freely" era closed.

> **Goal:** A deployer can run Maidan in production with clear readiness
> semantics, validated configuration, and a published checklist. API shape
> is treated as stable at `v1.0.0`.
>
> **Target tag:** `v1.0.0`.

## PRs

| #         | Title                                                          | Issue |
|-----------|----------------------------------------------------------------|-------|
| 1.0.1     | `feat(maidan-server): /health/live and /health/ready probes`   | TBD   |
| 1.0.2     | `docs: production runbook + config validation hardening`       | TBD   |
| 1.0.retro | `docs(retro): Cluster 1.0 retrospective + v1.0.0 tag prep`       | TBD   |

## Order

1. **1.0.1** — `/health/live` always 200; `/health/ready` runs DB + artifact
   checks (current `/health` behavior); keep `/health` as alias to ready.
2. **1.0.2** — `docs/Production.md`, README production section, stricter
   `Config::from_env` validation (`MAIDAN_ENV=production` forbids
   `AUTH_DISABLED`), Decisions entry for API stability at 1.0.
3. **1.0.retro** + `v1.0.0` tag.

## Exit criteria

- CI green.
- Kubernetes-style liveness/readiness documented.
- `docs/Retros/Cluster 1.0.md` merged; `v1.0.0` tagged.

## Out of scope

- Sigstore signing (Track V / X).
- OAuth/OIDC (post-1.0).
- Full OpenAPI generation (Track W).
