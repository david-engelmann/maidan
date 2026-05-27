# Production deployment

Guidance for running Maidan at `v1.0.0` and later. Security overview:
[[Threat-Model]].

## Probes

| Endpoint          | Use        | Behavior                                      |
|-------------------|------------|-----------------------------------------------|
| `GET /health/live`  | Liveness   | Always `200` if the process is up.            |
| `GET /health/ready` | Readiness  | `200` when DB, artifact store, indexer (if stale check enabled and no embedding errors), and Postgres `LISTEN` bus (when used) are healthy. |
| `GET /health`       | Readiness  | Alias of `/health/ready`.                     |

## Environment

| Variable        | Required | Notes                                                |
|-----------------|----------|------------------------------------------------------|
| `DATABASE_URL`  | yes      | Postgres (recommended) or SQLite.                    |
|                 |          | SQLite connections enable `foreign_keys`, WAL, and `busy_timeout=5000` ms automatically. |
| `MAIDAN_ENV`    | no       | Set to `production` to forbid `AUTH_DISABLED`.       |
| `AUTH_DISABLED` | no       | Must **not** be set in production.                   |
| `MAIDAN_BOOTSTRAP` | no    | Set to `1` only during initial seed when auth is on. Allows unauthenticated `POST /workspaces` and `POST /workspaces/:wid/members`. Only the **first** workspace may be created via bootstrap; remove the flag and restart after minting tokens. |
| `FEDERATION_ENCRYPTION_KEY` | when federation is used | 32-byte secret (base64 or hex) used to encrypt peer outbound bearer tokens at rest. Required to create peers and for the poll worker after restart. Back up with your DB; rotation requires re-creating peers. |
| `FEDERATION_DISABLED` | no | Set to `1` to disable the outbound poll worker. |
| `FEDERATION_POLL_INTERVAL_SECS` | no | Outbound poll interval (default `30`). |
| `MAIDAN_EMBEDDING_PROVIDER` | no | `hash-v1` (default) or `openai-compatible`. |
| `MAIDAN_EMBEDDING_ENDPOINT` | when provider is `openai-compatible` | Full URL to embeddings endpoint (OpenAI-compatible response shape). |
| `MAIDAN_EMBEDDING_MODEL` | when provider is `openai-compatible` | Embedding model id sent in request body. |
| `MAIDAN_EMBEDDING_API_KEY` | optional | Bearer token for remote provider. |
| `MAIDAN_EMBEDDING_DIM` | no | Expected embedding dimension (default `1024`). |
| `MAIDAN_EMBEDDING_TIMEOUT_SECS` | no | HTTP timeout for remote embeddings (default `15`). |

### Local embedding servers (e.g. LM Studio)

Maidan's indexer uses the **OpenAI-compatible embeddings** API shape, not chat
completion. Point `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` at your server's
**embeddings** URL (for example `http://localhost:1234/v1/embeddings`) and set
`MAIDAN_EMBEDDING_MODEL` to the loaded model id. A chat endpoint such as
`http://localhost:1235/api/v1/chat` is not used for search indexing.

## Bootstrap

When bearer auth is enabled, unauthenticated `POST /workspaces` and
`POST /workspaces/:wid/members` require `MAIDAN_BOOTSTRAP=1`. Only the **first**
workspace may be created via bootstrap; a second `POST /workspaces` returns
`403`.

Typical production seed (private network):

1. Set `MAIDAN_BOOTSTRAP=1` and `AUTH_DISABLED=1`.
2. Create workspace + member, mint admin token.
3. Unset both flags, set `MAIDAN_ENV=production`, restart.

Integration tests use `AUTH_DISABLED=1` (bootstrap flag not required).

Human browser login via OIDC ships in **`v2.0.0`**. See [[OIDC]] for design
detail. Summary:

| Variable | Required | Notes |
|----------|----------|-------|
| `MAIDAN_OIDC_ENABLED` | when using OIDC | `1` enables `/auth/oidc/*` and session routes. |
| `MAIDAN_SESSION_SECRET` | when OIDC on | HMAC key for signed `maidan_session` cookies (32+ bytes). Bare session UUIDs in cookies are rejected. |
| `MAIDAN_OIDC_ISSUER` | yes (non-mock) | IdP issuer URL for discovery. |
| `MAIDAN_OIDC_CLIENT_ID` | yes (non-mock) | OAuth client id. |
| `MAIDAN_OIDC_CLIENT_SECRET` | confidential clients | Code exchange secret. |
| `MAIDAN_OIDC_REDIRECT_URI` | yes | Registered callback (e.g. `https://host/auth/oidc/callback`). |
| `MAIDAN_OIDC_MOCK` | no | `1` for deterministic dev/CI only; forbidden when `MAIDAN_ENV=production`. |
| `MAIDAN_OIDC_FIRST_ADMIN` | no | Default on: session may mint the first `token:admin` per workspace via `POST /auth/session/mint`. Set `0` to disable. |
| `MAIDAN_COOKIE_SECURE` | no | Set `1` in production for `Secure` session cookies. |
| `MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI` | no | Registered post-logout redirect (e.g. `https://host/ui/`). Used when IdP exposes `end_session_endpoint`. |
| `MAIDAN_OIDC_AUTO_MINT` | no | `1` redirects to `/ui/?auto_mint=1` after login when the workspace has no `token:admin` yet; the UI then calls `POST /auth/session/mint`. Off by default. Requires first-admin mint (`MAIDAN_OIDC_FIRST_ADMIN` not `0`). |

After OIDC login, use `/ui/` (session cookie) or mint an API token for MCP.
Remove `MAIDAN_BOOTSTRAP` once the first human has `token:admin`.

## API discovery

| Endpoint            | Use                                      |
|---------------------|------------------------------------------|
| `GET /openapi.json` | Machine-readable OpenAPI 3.0 (Track W.1). HTTP routes and `application/problem+json` errors; MCP and WebSocket are not fully described. Auth/session routes are under the `auth` tag (`/auth/oidc/*`, `/auth/session`, `/ui/api/...`). |
| `GET /workspaces/:wid/search` | Search (`q`, optional `author` / `channel` / `kind`, `mode`). Default `mode=lexical`. `mode=semantic` embeds `q` with the configured provider and ranks by cosine similarity (**Postgres only**; `rank` is `1.0 - distance`, higher is better). Facets apply to both lexical and semantic modes on Postgres. On **Postgres** lexical `q`, `"phrase"`, `-word`, or `or` uses `websearch_to_tsquery`; plain words use `plainto_tsquery`. SQLite ignores web operators and rejects `mode=semantic`. |
| `GET /metrics`    | Prometheus text exposition (HTTP request counters + latency histogram). |
| `DELETE /messages/:id/purge` | Hard-delete a **tombstoned** message (GDPR erasure); requires bearer with `workspace:write`. |

Import into Swagger UI, Redoc, or your client generator. The document
version tracks the server release (`info.version`).

## API stability

From `v1.0.0`, HTTP and MCP shapes are semver-stable. Pre-1.0 releases
may break without migration shims.
