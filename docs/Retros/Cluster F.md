# Cluster F retro — Auth + capabilities

> Closing wave for Cluster F · target tag `v0.5.0`.

Cluster E left artifact and mutation APIs open on the network. Cluster F
adds workspace-scoped API tokens, a capability vocabulary, Bearer auth on
HTTP and MCP, token-in-frame WebSocket subscribe, and mint/revoke endpoints.

## What shipped

- **PR #76** — Cluster F kickoff plan + issues #68–#75
- **PR #77** — `feat(maidan-store): API token schema and store (F.1)`
- **PR #78** — `feat(cluster-f): auth, capabilities, and token API (F.2–F.7)`

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Cluster G | A2A transport auth                                | Separate cluster scope.                  |
| Cluster T | OTLP + request-id middleware                      | Telemetry track.                         |
| Post-1.0  | OAuth/OIDC human login                            | API tokens sufficient for agents v0.5.0. |
| Post-1.0  | Resumable WS session tokens                         | Cluster T / later.                       |
| Cluster F+| `sqlite-vec` semantic search on SQLite            | Extension immature.                      |

## Surprises

- **SQLite datetime compare** — RFC3339 `expires_at` strings beat
  `datetime('now')` lexicographically; active-token lookup binds `Utc::now()`.
- **Axum router state** — sub-routers must not each call `with_state`; merge
  then attach state once.
- **Bootstrap routes** — `POST /workspaces` and `POST …/members` stay outside
  Bearer middleware so greenfield setup works with `AUTH_DISABLED`, then mint.

## Decisions

- **Store SHA-256 hash only** — plaintext `maid_*` secret shown once at mint.
- **`AUTH_DISABLED=1`** — test harnesses pass `auth_disabled: true` on
  `AppState`; production reads env in `main`.
- **WS auth via subscribe frame** — HTTP upgrade stays unauthenticated; token
  validated in first text frame (close 1008 on failure).
- **Workspace bootstrap exempt** — not every route requires a pre-existing token.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Migration 0008 `maidan_api_tokens`                      | `v0.5.0`           |
| `maidan-auth` token hash + capability vocabulary        | `v0.5.0`           |
| HTTP Bearer middleware + 401/403 problem+json           | `v0.5.0`           |
| WS `SubscribeFrame.token` + `event:subscribe`           | `v0.5.0`           |
| MCP per-tool capability checks                          | `v0.5.0`           |
| `POST …/tokens` mint + `DELETE /tokens/:id` revoke      | `v0.5.0`           |

## Risks identified + mitigated

- **Breaking e2e tests** — `auth_disabled: true` in all existing harnesses.
- **Capability drift** — constants in `maidan-auth::capability`; MCP map in
  `maidan-mcp::tools::required_capability`.

## Risks identified + still open

- **Bootstrap routes unauthenticated** — workspace/member POST without Bearer;
  document prod flow: `AUTH_DISABLED` → seed → mint admin token → enable auth.
- **Artifact GET not workspace-scoped** — any token with `workspace:read` can
  fetch any sha256 known out-of-band.

## Forward look

Cluster G is agent-to-agent transport. Cut `v0.5.0` after this retro merges.

## Acknowledgements

Solo cluster. F.2–F.7 landed in one PR after F.1 proved the store contract.
