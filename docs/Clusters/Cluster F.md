# Cluster F — Auth, workspaces, capabilities

After Cluster E made artifacts first-class, Cluster F closes the
biggest standing security gap: every HTTP, WebSocket, and MCP entry
point is anonymous today. This cluster adds API tokens bound to
members, a capability vocabulary, and enforcement on mutations.

> **Goal:** Callers present a bearer token; the server resolves it to a
> `member_id` + capability set and rejects unauthorized actions with 401/403
> RFC 7807 problems. Dev/test can disable auth via `AUTH_DISABLED=1`.
>
> **Target tag:** `v0.5.0`.

## PRs

| #       | Title                                                                 | Issue |
|---------|-----------------------------------------------------------------------|-------|
| F.1     | `feat(maidan-store): schema 0008 api tokens + capabilities`           | #68   |
| F.2     | `feat(maidan-auth): token hash, validate, Capability model`           | #69   |
| F.3     | `feat(maidan-server): Bearer auth middleware on HTTP`                 | #70   |
| F.4     | `feat(maidan-server): auth on WebSocket subscribe`                    | #71   |
| F.5     | `feat(maidan-server): auth on MCP POST /mcp`                            | #72   |
| F.6     | `feat(maidan-server): capability checks on mutations`                 | #73   |
| F.7     | `feat(maidan-server): token mint and revoke API`                        | #74   |
| F.retro | `docs(retro): Cluster F retrospective + v0.5.0 tag prep`               | #75   |

## Order

1. **F.1** — migration 0008: `maidan_api_tokens` (`token_hash`, `member_id`,
   `workspace_id`, `capabilities` JSON, `expires_at`, `revoked_at`). Store
   trait methods: `create_token`, `get_token_by_hash`, `revoke_token`.
2. **F.2** — `maidan-auth`: `Capability` enum / string newtype, `TokenSecret`
   generation, SHA-256 hash, constant-time compare, `AuthContext` resolved
   from bearer string.
3. **F.3** — axum middleware: `Authorization: Bearer <token>` on all routes
   except `/health`; `AUTH_DISABLED` for existing e2e tests.
4. **F.4** — `SubscribeFrame` carries `token`; validate before bus attach.
5. **F.5** — MCP handler reads same Bearer header; reject unauthenticated
   `tools/call` / `resources/read`.
6. **F.6** — map routes to required capabilities (e.g. `message:post`,
   `artifact:upload`, `thread:transition`); return 403 when member lacks scope.
7. **F.7** — `POST /workspaces/:wid/members/:mid/tokens` returns plaintext
   secret once; `DELETE /tokens/:id` revokes.
8. **F.retro** + `v0.5.0` tag.

F.3–F.5 can land in one PR if review overhead is high, but the plan keeps
them separate for bisectability.

## Capability vocabulary (v0.5.0)

| Capability          | Allows                                      |
|---------------------|---------------------------------------------|
| `workspace:read`    | GET workspaces, channels, threads, messages |
| `workspace:write`   | POST/PATCH entities, references, votes        |
| `message:post`      | `POST .../messages`                         |
| `thread:transition` | `POST /threads/:id` FSM actions             |
| `artifact:upload`   | `POST /artifacts`                           |
| `search:query`      | `GET .../search`, MCP `search_messages`     |
| `event:subscribe`   | WebSocket `/ws/subscribe`                   |
| `token:admin`       | Mint/revoke tokens for workspace members    |

Default token minted in F.7 tests includes `workspace:read` + `workspace:write`
+ `event:subscribe` + `search:query`.

## Exit criteria

- CI green on `main`.
- With `AUTH_DISABLED` unset, unauthenticated `POST /messages` returns 401.
- Valid bearer can post; token without `message:post` gets 403.
- WS without token closes with policy violation; with token streams events.
- MCP `tools/call` without Bearer fails; with token succeeds on allowed tool.
- Token mint returns secret once; revoked token fails immediately.
- [[Retros/Cluster F]] merged; `v0.5.0` tagged.

## Risks

| Risk                                                                 | Mitigation                                                                 |
|----------------------------------------------------------------------|----------------------------------------------------------------------------|
| Breaking every e2e test at once                                        | `AUTH_DISABLED=1` default in test harness; compose documents prod setting. |
| Storing plaintext tokens                                             | Store SHA-256 hash only; show secret once at mint.                         |
| Capability matrix drift from routes                                  | F.6 central table in `maidan-auth` or server `authz.rs`.                 |
| SQLite vs Postgres JSON for capabilities                             | F.1 uses `TEXT` JSON on both dialects.                                   |
| Over-scoping multi-tenant “orgs”                                     | Defer org-level tenancy; tokens are workspace-scoped in v0.5.0.          |

## Out of scope (deferred)

- OAuth/OIDC / SSO human login (post-1.0).
- Resumable WS session tokens (Cluster T).
- `sqlite-vec` semantic search (Cluster F+ / when extension matures).
- Thread reopen FSM edges (post-1.0).
