# Cluster 2.0 — OIDC identities and human sessions

After optional minors `v1.1.0`–`v1.4.0`, Maidan's next major step is
first-class human login while preserving bearer-token flows for agents.
This cluster implements runtime OIDC, session handling, and identity mapping
designed in [[OIDC]].

> **Goal:** Human users can authenticate via OIDC authorization-code + PKCE,
> receive a secure Maidan session, and operate in a workspace without
> exposing long-lived bearer secrets in browser JavaScript.
>
> **Target tag:** `v2.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 2.0.1 ✓   | `feat(maidan-store): oidc identities + session schema`                 | #133  |
| 2.0.2 ✓   | `feat(maidan-server): OIDC login/callback/logout routes`              | #134  |
| 2.0.3 ✓   | `feat(maidan-server): session middleware + /auth/session`             | #135  |
| 2.0.4 ✓   | `feat(maidan-server): UI/session integration + first-human admin policy` | #136 |
| 2.0.retro ✓ | `docs(retro): Cluster 2.0 retrospective + v2.0.0 tag prep`          | TBD   |

## Order

1. **2.0.1** — add `maidan_oidc_identities` and session persistence model
   (or explicitly cookie-only session decision) with tests on Postgres/SQLite
   parity where applicable.
2. **2.0.2** — implement OIDC provider discovery and auth-code+PKCE
   callback exchange, including `state`/`nonce` validation and workspace
   binding in callback state.
3. **2.0.3** — add session middleware and `GET /auth/session` surface;
   keep existing bearer middleware unchanged on MCP/A2A routes.
4. **2.0.4** — wire `/ui/` and operator workflows to session context,
   define first-human admin/token mint policy, and document migration path.
5. **2.0.retro** + `v2.0.0` tag.

## Exit criteria

- CI green on `main`.
- OIDC login succeeds against a test IdP fixture (or deterministic mock).
- Session cookie uses secure defaults (`HttpOnly`, `Secure`, `SameSite=Lax`)
  and callback rejects invalid `state`/`nonce`.
- Existing bearer-token workflows (HTTP/MCP/WS) remain functional and covered.
- [[Retros/README]] includes Cluster 2.0; `v2.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Scope creep from auth rewrite | Keep bearer surfaces stable; add session paths incrementally. |
| First-admin bootstrap ambiguity | Make policy explicit in 2.0.4; document break-glass operator flow. |
| IdP-specific behavior drift | Rely on standards-compliant discovery + minimal provider assumptions. |

## Out of scope

- SCIM/group sync and enterprise provisioning.
- Device-code flow for MCP clients.
- Replacing API tokens for automation.
