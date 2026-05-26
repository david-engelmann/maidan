# Cluster 2.1 retro — OIDC operator hardening

> Closing wave for Cluster 2.1 · target tag `v2.1.0`.

Cluster 2.0 shipped runtime OIDC and browser sessions. Cluster 2.1 closes
operator gaps from [[Retros/Cluster 2.0]]: signed cookies, IdP logout,
OpenAPI auth routes, and optional first-login auto-mint.

## What shipped

- **PR #139** — HMAC-signed `maidan_session` cookies (`uuid.hmac`)
- **PR #142** — IdP `end_session_endpoint` redirect on logout
- **PR #141** — OpenAPI for `/auth/*` and `sessionCookie` security scheme
- **PR #143** — `MAIDAN_OIDC_AUTO_MINT` + `/ui/` polish (copy secret, session UX)

## What was deferred

| To           | What                              | Why                                      |
|--------------|-----------------------------------|------------------------------------------|
| Cluster 3.0  | Semantic search facets            | Search/subscriber cluster on deck.         |
| Post-2.1     | Session encryption at rest        | Cookie is opaque id only; rows unencrypted. |
| Post-2.1     | SCIM / enterprise group sync        | Out of scope for minors.                 |

## Surprises

- **Stacked PR #140** closed when 2.1.1 merged; 2.1.2 cherry-picked cleanly onto `main`.
- **Browser logout** must POST (form submit) so IdP end-session redirects work; `fetch` does not navigate.
- **Auto-mint** uses `?auto_mint=1` hint on redirect — keeps minted secrets out of URLs.

## Decisions

- **Signed cookie format** — `session_id.hmac`; bare UUID cookies rejected when OIDC is on.
- **Auto-mint off by default** — `MAIDAN_OIDC_AUTO_MINT=1` only when operators accept UI-driven mint.
- **OpenAPI version** — bumped to `2.1.0` in server OpenAPI metadata for this cluster.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| HMAC-signed `maidan_session` cookie                     | `v2.1.0`           |
| IdP end-session redirect on `POST /auth/logout`         | `v2.1.0`           |
| OpenAPI `auth` tag + `sessionCookie` scheme             | `v2.1.0`           |
| `MAIDAN_OIDC_AUTO_MINT` + `/ui/?auto_mint=1` flow       | `v2.1.0`           |

## Risks identified + mitigated

- **Forged session cookies** — HMAC with `MAIDAN_SESSION_SECRET`.
- **Token in redirect URL** — auto-mint triggers client-side `POST /auth/session/mint`, not server redirect with secret.

## Risks identified + still open

- **`rsa` timing advisory** — unchanged from 2.0 (`deny.toml` ignore).
- **Auto-mint in shared browsers** — one-time banner + docs warn operators.

## Forward look

**Cluster 3.0** — semantic search facets, coverage CI gate, WS gap auto-replay
(see [[Clusters/Cluster 2.1]] on-deck sketch).

## Acknowledgements

Solo cluster. Four implementation PRs plus this retro.
