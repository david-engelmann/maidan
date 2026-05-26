# Cluster 2.0 retro — OIDC identities and human sessions

> Closing wave for Cluster 2.0 · target tag `v2.0.0`.

Cluster 1.4 planned OIDC; Cluster 2.0 delivers runtime login, server-side
sessions, and browser UI integration while keeping MCP/A2A on bearer tokens.

## What shipped

- **PR #133** — `feat(maidan-store): oidc identities + session schema` (migration 0012)
- **PR #134** — `feat(maidan-server): OIDC login/callback/logout routes` (PKCE + mock IdP)
- **PR #135** — `feat(maidan-server): session middleware + GET /auth/session`
- **PR #136** — `feat(maidan-server): UI/session integration + first-human admin policy`

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Post-2.0  | IdP end-session redirect on logout                  | MVP clears Maidan session only.          |
| Post-2.0  | Auto-mint narrow UI token without explicit click    | OIDC.md option (b); explicit mint is safer. |
| Post-2.0  | SCIM / enterprise group sync                        | Out of cluster scope.                    |
| Post-2.0  | Device-code flow for MCP                            | Agents stay on API tokens.               |

## Surprises

- **openidconnect v4** dropped `reqwest::async_http_client`; use `reqwest::Client` with redirect policy `none`.
- **`CoreClient` endpoint type parameters** must be spelled out (`ConfiguredOidcClient`) to store a discovered client.
- **`RUSTSEC-2023-0071`** on transitive `rsa` — ignored in `deny.toml` (no fixed release).
- **Session-derived `AuthContext`** needs explicit read caps for `/ui/api/...` routes, not empty capabilities.

## Decisions

- **Mock OIDC for CI** — `MAIDAN_OIDC_MOCK=1` forbidden in production; `oidc_e2e.rs` exercises full cookie flow.
- **Session table on Postgres + SQLite** — parity with store tests; cookie holds opaque `SessionId` only.
- **First admin mint** — `POST /auth/session/mint` when no active `token:admin` in workspace; opt-out via `MAIDAN_OIDC_FIRST_ADMIN=0`.
- **Separate `/ui/api/` routes** — `session_or_bearer_middleware` without changing MCP bearer middleware.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| `maidan_oidc_identities` + `maidan_sessions` schema     | `v2.0.0`           |
| OIDC login / callback / logout                          | `v2.0.0`           |
| `maidan_session` HttpOnly cookie                        | `v2.0.0`           |
| `GET /auth/session`                                     | `v2.0.0`           |
| `POST /auth/session/mint` (first `token:admin`)           | `v2.0.0`           |
| `/ui/` OIDC + session event tail                        | `v2.0.0`           |
| `GET /ui/api/workspaces/:wid/events` (session or bearer) | `v2.0.0`          |

## Risks identified + mitigated

- **CSRF on OIDC** — random `state` + one-time pending row in DB.
- **Production mock IdP** — rejected at config load when `MAIDAN_ENV=production`.
- **Bearer regression on MCP** — unchanged `auth::middleware` on `/mcp`.

## Risks identified + still open

- **`rsa` timing advisory** — documented ignore; revisit when openidconnect upgrades.
- **Email claim trust** — auto-link only when `email_verified` and `MAIDAN_OIDC_LINK_EMAIL=1`.
- **Session fixation** — new session row on each login; no rotation doc yet.

## Forward look

Post-2.0: operator hardening (session rotation, IdP logout), optional UI token
auto-mint, and Postgres-first session encryption at rest if required.

## Acknowledgements

Solo cluster. Design spike from `v1.4.2` (`docs/OIDC.md`) carried straight
into four implementation PRs plus this retro.
