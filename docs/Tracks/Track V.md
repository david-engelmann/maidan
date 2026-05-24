# Track V — Security + privacy

Cross-cutting track (no version tag). Hardens production posture after
`v1.0.0` without breaking the stable HTTP/MCP API.

> **Goal:** Documented threat model, erasure flow, and signed release
> artifacts.
>
> **Not a cluster** — ship as numbered PRs on `main`.

## PRs (proposed)

| #    | Title | Source |
|------|-------|--------|
| V.1  | `docs: threat model + bootstrap hardening options` | Standing risks |
| V.2  | `feat(maidan-store): GDPR hard-delete past tombstone` | Cluster A retro |
| V.3  | `ci: cosign sign release binaries` | Cluster A retro |
| V.4  | `chore(k8s): NetworkPolicy manifests` | Cluster A retro |

## Exit criteria

- Threat model reviewed and linked from [[Production]].
- Erasure API or admin tool documented with audit implications.
- Release workflow verifies signatures (or documents manual step).

## Out of scope

- OAuth/OIDC (post-1.0 product; breaking auth surface → `v2.0.0`).
