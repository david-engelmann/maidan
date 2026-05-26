# Minor 1.4 retro — Auth hardening

> Closing wave for optional minor **`v1.4.0`** · [[Post-1.0]] ladder 1.4.1–1.4.2.

This minor tightens bootstrap exposure and captures the OIDC plan without
breaking the stable bearer-token API: one-shot bootstrap gating in runtime,
and a design-only OIDC spike deferred to `v2.0.0`.

## What shipped

| PR   | Scope |
|------|-------|
| #129 | `MAIDAN_BOOTSTRAP=1` gate on unauthenticated bootstrap routes + one-shot first workspace enforcement. |
| #130 | OIDC human login design spike (`docs/OIDC.md`) and ADR deferring runtime OIDC to `v2.0.0`. |

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| `v2.0.0`  | Runtime OIDC login/session middleware             | Breaking auth/session surface.           |
| Open Work | Coverage minimum % gate / Codecov integration     | Existing artifact only; no policy gate.  |
| Open Work | Semantic facets (`mode=semantic` + author/channel/kind) | Ranking/filter semantics still open. |
| Open Work | Per-model embedding tables / mixed dimensions     | Schema + query contract design pending.  |

## Surprises

- **Bootstrap one-shot is stateful, not one-process** — counting existing
  workspaces was the simplest robust gate and works across restarts.
- **OIDC planning exposed first-admin UX tension** — the first human login
  and `token:admin` mint policy need a dedicated v2.0 decision.
- **Threat model drift surfaces fast** — once bootstrap behavior changed,
  security docs needed immediate reconciliation across Production/Open Work.

## Decisions

- **Bootstrap remains route-scoped** — only `POST /workspaces` and
  `POST /workspaces/:wid/members` stay unauthenticated, now behind
  `MAIDAN_BOOTSTRAP=1` when auth is enabled.
- **One-shot enforcement at workspace creation** — while bootstrap is on,
  a second `POST /workspaces` returns `403`; member creation can continue
  until operators disable bootstrap.
- **OIDC is design-only in v1.4** — runtime OIDC lands in `v2.0.0` to avoid
  partial session auth in a minor release.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Bootstrap routes require `MAIDAN_BOOTSTRAP=1` when auth is enabled | `v1.4.0` |
| One-shot bootstrap workspace seed enforcement           | `v1.4.0`           |
| OIDC human login architecture/design spike (`docs/OIDC.md`) | `v1.4.0`       |

## Risks identified + still open

- **At-most-once event bus delivery** remains (replay helps but not exactly-once).
- **`AUTH_DISABLED` remains a high-impact flag** if left on outside controlled seed windows.
- **Runtime OIDC/session security** (cookie theft/CSRF/claim mapping) remains deferred to `v2.0.0`.

## Forward look

Next major delivery focus is **`v2.0.0`**: OIDC runtime implementation
(login/callback/session), identity mapping, and first-human admin policy,
while preserving bearer-token flows for MCP and automation.

## Acknowledgements

Solo minor. Two PRs, one retro, tag `v1.4.0`.
