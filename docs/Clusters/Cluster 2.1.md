# Cluster 2.1 — OIDC operator hardening

Cluster 2.0 shipped runtime OIDC, server-side sessions, and a minimal `/ui/`.
This cluster closes the post-2.0 operator gaps from [[Retros/Cluster 2.0]] and
[[OIDC]] without changing MCP/A2A bearer auth.

> **Goal:** Production operators can trust session cookies, log out through the
> IdP when configured, discover auth routes in OpenAPI, and optionally
> auto-mint the first admin token after login.
>
> **Target tag:** `v2.1.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 2.1.1     | `feat(maidan-server): HMAC-signed session cookies`                     | TBD   |
| 2.1.2     | `feat(maidan-server): IdP end-session redirect on logout`              | TBD   |
| 2.1.3     | `feat(maidan-server): OpenAPI for auth/session routes`                 | TBD   |
| 2.1.4     | `feat(maidan-server): optional OIDC auto-mint + UI polish`             | TBD   |
| 2.1.retro | `docs(retro): Cluster 2.1 retrospective + v2.1.0 tag prep`            | TBD   |

## Order

1. **2.1.1** — bind `MAIDAN_SESSION_SECRET` to HMAC-signed cookie values
   (`session_id.mac`); reject unsigned/forged cookies; update `oidc_e2e`.
2. **2.1.2** — on logout, if IdP exposes `end_session_endpoint`, redirect
   browser after clearing Maidan session (optional `post_logout_redirect_uri`).
3. **2.1.3** — document `/auth/*` and session mint in OpenAPI + Production.
4. **2.1.4** — `MAIDAN_OIDC_AUTO_MINT=1` redirects to mint after first login;
   `/ui/` shows login state and copy-to-clipboard for minted secret.
5. **2.1.retro** + `v2.1.0` tag.

## Exit criteria

- CI green on `main`.
- Forged or bare-UUID session cookies are rejected when OIDC is enabled.
- Logout clears Maidan session; IdP logout redirect when discovery provides endpoint.
- OpenAPI lists auth/session routes; env table updated in [[Production]].
- [[Retros/README]] includes Cluster 2.1; `v2.1.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Cookie format change logs everyone out | Acceptable on minor; note in CHANGELOG. |
| IdP logout variance | Optional redirect; fall back to local-only logout. |
| Auto-mint UX leaks token in browser history | One-time display + warn in docs; keep flag off by default. |

## Out of scope

- SCIM / enterprise provisioning.
- Device-code OIDC for MCP.
- Session encryption at rest (Postgres row still opaque id).
- Semantic search facets → candidate **Cluster 3.0** ([[Open Work]]).

## Alternative next cluster (not this wave)

**Cluster 3.0 — Search & subscriber depth** (`v3.0.0`): semantic search facets,
coverage CI gate, WS gap auto-replay. Schedule after 2.1 unless priorities shift.
