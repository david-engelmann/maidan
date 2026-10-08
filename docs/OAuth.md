# OAuth Authorization Server

Maidan is becoming an OAuth 2.1 authorization server (Next 23, decided
2026-10-08), so connected-app directories can authenticate users against it.
The research is archived at
`docs/archive/OAuth authorization server research 2026-10-08.md`.

## Design decisions (2026-10-08)

- **Build, don't adopt.** Maidan already owns ~70% of an AS (PKCE auth-code
  flow in `app_oauth.rs`, persisted codes, capability-scoped token minting).
  No Rust crate fits the capability model.
- **Full OAuth 2.1 scope**, seven phases, P0–P1 landing first as a reviewable
  milestone.
- **Consent UX lives in the `/ui` console** (the trust root).
- **Public and confidential clients** from day one.
- **No DCR** (deprecated by spec); CIMD + pre-registered clients only.
- **Capabilities are the OAuth scopes** — no parallel ACL.

## Phase plan

| Phase | What ships | Size | Acceptance criteria | Depends on |
|---|---|---|---|---|
| **P0: Resource-server compliance** | `WWW-Authenticate` on MCP 401s; `/.well-known/oauth-protected-resource` (RFC 9728) | XS (1–2 days) | An unauthenticated `GET /mcp` returns 401 with `WWW-Authenticate: Bearer resource_metadata="..."`; the metadata document validates against RFC 9728 | Nothing |
| **P1: AS metadata** | `/.well-known/oauth-authorization-server` (RFC 8414) | XS (1 day) | The document validates against RFC 8414; it advertises no grant type, endpoint, or registration method that is not served | P0 |
| **P2: Client registry** | `maidan_oauth_clients` table + CRUD routes + CIMD fetcher | S (3–4 days) | Pre-registered clients persist; CIMD documents fetch through the egress guard with SSRF protection; client IDs resolve | P1 |
| **P3: Authorize + consent** | `GET /oauth/authorize`, consent screen in `/ui`, code minting (generalizes `app_oauth.rs`) | M (1 week) | PKCE S256 required for public clients; exact redirect-URI match; consent page cannot be framed; grants never exceed the member's capabilities | P2, OIDC session |
| **P4: Token endpoint** | `POST /oauth/token` (code exchange + refresh rotation), `POST /oauth/revoke` | M (1 week) | Refresh rotation per OAuth 2.1 §4.3.1 (reuse revokes the family); RFC 8707 resource binding; `iss` on responses (RFC 9207) | P3 |
| **P5: Consent management** | `maidan_oauth_consents` table, console UI for grant/revoke | S (3 days) | Users can list and revoke their grants; revocation takes effect immediately | P3 |
| **P6: Hardening** | Rate limits, SSRF guards, audit coverage, scope contract tests | S (3–4 days) | Metadata fetched only through the egress guard; contract tests fail on unscoped grants | P4 |
| **P7: Validation record** | End-to-end against MCP clients with real OAuth (feeds Next 14) | S (2–3 days) | Full flow works against a public instance; record names the commit | P6, public instance |

**Total:** ~5–6 weeks.

## Phase dependencies

P0–P1 ship independently (this PR). P2–P7 wait for Next 17 (grokbot):
a model holding an OAuth token must not be able to accept gates until the
credential rule lands.

## Code layout

New code lives in `crates/maidan-server/src/oauth/`, not `app_oauth.rs`,
so the OAuth AS and the installed-app flow touch disjoint files:

- `oauth/mod.rs` — module root
- `oauth/metadata.rs` — RFC 8414 + RFC 9728 discovery documents
- (later) `oauth/authorize.rs`, `oauth/token.rs`, `oauth/clients.rs`,
  `oauth/consent.rs`

Migrations for this track use numbers 0150–0159 (grokbot holds 0147–0149).
