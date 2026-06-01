# Cluster 57.0 — Agent app model

**Theme:** OAuth-style installed apps with scoped capabilities separate from member tokens.

## Problem

API tokens are always minted for a concrete `member_id`. External integrations (MCP
clients, automation services) need an **app identity** with its own capability grant,
not a human/agent member's personal token.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Schema | `maidan_apps`, `maidan_app_installations`; `api_tokens.app_installation_id` |
| Store | Register app, install (bot `MemberKind::Agent`), mint/revoke app tokens |
| Auth | Bearer resolves app tokens; reject when installation revoked |
| HTTP | App CRUD + install + `POST .../app-installations/:iid/tokens` |
| Tests | E2E: app token posts as bot; caps subset enforced |
| Docs | Retro, CHANGELOG `v57.0.0`, Capabilities |

## PR ladder

| # | Title |
|---|--------|
| 57.0.1 | `feat(store): apps + installations schema and store` |
| 57.0.2 | `feat(server): app install and app token HTTP API` |
| 57.0.3 | `test(server): agent app e2e` |
| 57.retro | `docs(retro): Cluster 57.0 + v57.0.0` |

## Exit criteria

- Operator registers an app, installs it (bot member created), mints a token with a
  subset of granted capabilities.
- App token cannot exceed installation grant; member token mint path unchanged.
- `v57.0.0` tagged after retro.

## Out of scope

- OAuth authorization-code flow / redirect URIs (install is operator-authenticated).
- App marketplace or cross-workspace apps.
- Per-app rate-limit keys separate from token quotas (reuse `maidan_token_quotas`).

## Tag

`v57.0.0`

See [[Clusters/Product Ladder 35+]] Phase VII.
