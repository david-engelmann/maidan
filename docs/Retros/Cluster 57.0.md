# Cluster 57.0 retro — Agent app model

> Tag **`v57.0.0`**.

## What shipped

- `maidan_apps` + `maidan_app_installations`; bot member per install (`app:{slug}`).
- App tokens on `maidan_api_tokens.app_installation_id` with subset capability checks.
- HTTP CRUD for apps, install, revoke, mint app token.

## What was deferred

- OAuth authorization-code install flow for third-party clients.
- MCP tools mirroring app admin routes.

## Forward look

Cluster **58**: Maidan 2.0 completion gate → **`maidan-2.0`** checklist (see [[Retros/Product Ladder 35+]]).
