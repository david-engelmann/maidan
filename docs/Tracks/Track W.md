# Track W — Documentation

Cross-cutting track (no version tag). Makes the `v1.0.0` API discoverable
for integrators.

> **Goal:** Generated OpenAPI + published docs site consuming this vault.
>
> **Not a cluster** — ship as numbered PRs on `main`.

## PRs (proposed)

| #    | Title | Source |
|------|-------|--------|
| W.1  | `feat(maidan-server): OpenAPI via utoipa` | Cluster B retro |
| W.2  | `docs: mdBook site + CI publish` | Cluster H retro |
| W.3  | `docs: MCP tool reference from maidan-mcp` | Implicit |

## Exit criteria

- `/openapi.json` served in dev; stable schema for documented routes.
- Site build in CI (artifact or GitHub Pages).
- Vault index links to generated API reference.

## Out of scope

- Replacing Obsidian as the design source of truth.
