# Maidan OAuth Authorization Server — Architecture Research

> **A research snapshot, kept as written on 2026-10-08.** The plan is `docs/OAuth.md`, and where they disagree the plan is right: phase numbering, the resource identifier (`<origin>/mcp/streamable`), and when the authorization-server document is served all changed after this was written. What Maidan does today is in `docs/Claims.md`. In particular, the installed-app flow's PKCE is optional, not required, and its code exchange consumes the code before checking the verifier, which the plan's acceptance criteria correct.

**Date:** 2026-10-08
**Decision context:** The maintainer (2026-10-08) asked for full research before deciding on the OAuth AS revisit. "I want it perfectly architectured and planned out, add tools needed to local setup and document what would be needed to make it real."

---

## 1. Executive Summary

**Recommendation: BUILD, don't adopt.** Maidan already has much of what an OAuth authorization server needs. What is missing is protocol surface (endpoints, metadata, consent), not cryptography or token machinery. No existing Rust crate fits: `oxide-auth` is actix-oriented, OAuth 2.0 (not 2.1), has no axum frontend, and can't mint Maidan's capability-scoped tokens. Building on the existing `app_oauth.rs` foundation is smaller, safer, and preserves Maidan's audit and capability invariants.

**What "done" looks like:** Maidan becomes an OAuth 2.1 authorization server (RFC 8414 metadata, PKCE-only code flow, CIMD client registration, refresh rotation) while remaining its own resource server. The MCP endpoint advertises RFC 9728 protected-resource metadata so MCP clients auto-discover the AS. Capabilities become the OAuth scope vocabulary — no parallel ACL.

---

## 2. Current State: What Maidan Already Has

| Component | Status | Location |
|---|---|---|
| OIDC relying party (auth code + PKCE) | ✅ Shipped | `crates/maidan-server/src/oidc/` |
| OAuth-style auth code flow for installed apps | ✅ Shipped | `crates/maidan-server/src/app_oauth.rs` |
| PKCE S256 verification | ✅ Shipped | `app_oauth.rs::s256_challenge` |
| Persisted, hashed, single-use, TTL'd auth codes (cross-replica) | ✅ Shipped | `maidan_oauth_codes` table, `OAuthCodeStore` trait (pg + sqlite) |
| Token minting with capability scoping | ✅ Shipped | `routes/token.rs::mint_api_token` |
| Token attenuation (holder-side, no amplification) | ✅ Shipped | `routes/token.rs::attenuate_api_token` |
| Delegation grants (short-lived, one-hop) | ✅ Shipped | `routes/token.rs`, `from_delegated_token` |
| Token revocation (instant, audited) | ✅ Shipped | `revoke_api_token` |
| Token rotation | ✅ Shipped | `rotate_api_token` |
| Audit trail on every token operation | ✅ Shipped | `*_audited` store methods |
| Anonymous read-only dev mode for MCP | ✅ Shipped | `AuthContext::anonymous_reader` |
| App installation model (bot member + granted caps) | ✅ Shipped | `maidan_apps`, `app_installations` |

### The `app_oauth.rs` flow (today)

1. `POST /oauth/app/token/authorize` — requires `token:admin`, takes `redirect_uri` + `state` + optional `code_challenge`, mints a 10-min code (hashed in DB).
2. `POST /oauth/app/token` (public) — exchanges code (+ `code_verifier` if PKCE) for an app-scoped API token bound to the installation's bot member.

This is **workspace-internal**: an admin authorizes, not the end user. There's no consent screen, no public client registry, no refresh tokens, no standard endpoints.

---

## 3. What the Spec Requires (MCP 2026-07-28 Authorization Profile)

From the MCP authorization spec (2026-07-28 revision):

| # | Requirement | Level |
|---|---|---|
| 1 | AS **MUST** implement OAuth 2.1 (auth code + PKCE only; no implicit, no password grant) | MUST |
| 2 | AS and clients **SHOULD** support Client ID Metadata Documents (CIMD, draft-ietf-oauth-client-id-metadata-document-00) | SHOULD |
| 3 | AS and clients **MAY** support Dynamic Client Registration (RFC 7591) — **deprecated**, kept for compat | MAY |
| 4 | MCP server **MUST** implement Protected Resource Metadata (RFC 9728) | MUST |
| 5 | AS **MUST** provide RFC 8414 and/or OIDC Discovery metadata | MUST |
| 6 | Clients **MUST** send RFC 8707 `resource` on authorize **and** token requests | MUST |
| 7 | Resource server **MUST** validate token was issued for *that* resource (no passthrough) | MUST |
| 8 | Refresh token rotation is a **MUST** for public clients (OAuth 2.1 §4.3.1) | MUST |
| 9 | AS **SHOULD** return `iss` (RFC 9207) in authorization response | SHOULD |

**Client registration precedence:** pre-registered → CIMD → DCR → manual.

**What MCP clients actually do:**
- MCP clients typically prefer **CIMD**; DCR and pre-registered clients are also supported.
- Clients prefer CIMD when the AS advertises `client_id_metadata_document_supported`.
- CLI-based clients use loopback redirects on ephemeral ports — the AS must accept `http://localhost/...` redirects regardless of port.

---

## 4. Architecture Recommendation: Build on `app_oauth.rs`

### 4.1 Why not `oxide-auth` (or another crate)

1. **Wrong framework.** `oxide-auth` ships actix/rocket/iron/rouille frontends. Maidan is axum. Writing an axum frontend means implementing `WebRequest`/`WebResponse` traits — the hard part anyway.
2. **Wrong protocol version.** It's OAuth 2.0, not 2.1. We'd still hand-roll PKCE-required flows, refresh rotation, and `iss` parameters.
3. **Wrong token model.** Maidan's tokens are capability-scoped, workspace-bound, audited, revocable bearers resolved via `AuthContext`. No crate understands this. The token *minting* is the easy part; the *integration* is the work.
4. **We already have the hard parts.** Persisted auth codes, PKCE, token lifecycle, audit — all production. The gap is HTTP surface, not machinery.

### 4.2 Proposed architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    maidan-server (axum)                      │
│                                                             │
│  NEW: crates/maidan-server/src/oauth/                        │
│  ├── authorize.rs    GET  /oauth/authorize  (browser flow)   │
│  ├── token.rs        POST /oauth/token      (code+refresh)   │
│  ├── revoke.rs       POST /oauth/revoke      (RFC 7009)      │
│  ├── metadata.rs     GET  /.well-known/oauth-authorization-server (RFC 8414)│
│  ├── resource.rs     GET  /.well-known/oauth-protected-resource  (RFC 9728) │
│  ├── consent.rs      Consent screen handlers (session-auth)  │
│  ├── clients.rs      Client registry CRUD (token:admin)      │
│  └── cimd.rs         Client ID Metadata Document fetcher    │
│                                                             │
│  REUSE:                                                        │
│  ├── app_oauth.rs    Code mint/consume logic → generalized   │
│  ├── maidan-auth     TokenSecret, hash_secret, AuthContext   │
│  ├── routes/token.rs mint/attenuate/revoke patterns         │
│  └── oidc/           Session auth for the authorize endpoint │
└─────────────────────────────────────────────────────────────┘
```

### 4.3 The flows

**Authorization code + PKCE (public clients: browser and CLI-based MCP clients)**

```
1. Client discovers AS via /.well-known/oauth-protected-resource on /mcp
2. Client GET /oauth/authorize?
     response_type=code
     &client_id=https://client.example.com/metadata (CIMD URL) or pre-registered id
     &redirect_uri=https://.../callback
     &scope=workspace:read+search:query
     &state=...
     &code_challenge=...&code_challenge_method=S256
     &resource=https://maidan.example.com/mcp          (RFC 8707)
3. Maidan: validate client (pre-registered or fetch CIMD), validate redirect_uri
   against registered set (loopback: allow any port per RFC 8252 §7.3)
4. User authenticates (existing OIDC session or browser session)
5. Consent screen: "The client wants: read workspace, search" [Allow] [Deny]
   → creates a per-user, per-client grant (like app_installation but for OAuth)
6. Mint auth code (10 min, single-use, hashed, PKCE-bound, resource-bound)
   → 302 to redirect_uri?code=...&state=...&iss=https://maidan.example.com (RFC 9207)
7. Client POST /oauth/token (form-encoded):
     grant_type=authorization_code&code=...&redirect_uri=...
     &code_verifier=...&resource=https://maidan.example.com/mcp
     &client_id=... (public: no secret; confidential: HTTP Basic)
8. Maidan: consume code (atomic), verify PKCE, verify resource match,
   mint access token (opaque maid_..., 1h) + refresh token (rotating, 30d)
9. Client uses access token as Bearer <redacted> /mcp
10. On expiry: POST /oauth/token grant_type=refresh_token → new access +
    new refresh (old refresh invalidated — rotation)
```

**Key design decisions:**

| Decision | Choice | Rationale |
|---|---|---|
| Access token format | **Opaque** (`maid_...`, DB-validated) | Instant revocation; Maidan is both AS and RS so no JWT needed; matches existing `resolve_bearer` path; zero new validation code |
| Scope vocabulary | **Maidan capabilities** (`workspace:read`, `search:query`, …) | No parallel ACL (Protocols.md J6 explicitly forbids a second ACL); `scope` param carries space-delimited capability names |
| Non-delegatable caps | Never grantable via OAuth | `token:admin`, `operator:global`, `audit:read-global` excluded — same rule as `mint_vocabulary` |
| Client types | Public (PKCE, no secret) + confidential (secret, hashed) | browser-based clients are public; server-side integrations can be confidential |
| Client registration | Pre-registered (DB) + CIMD | DCR deprecated by spec; CIMD is what MCP clients prefer |
| Consent | Per user × client × scope-set, remembered | "Allow once" / "Allow always"; revocable from console |
| Refresh | Rotating, single-use, 30d, bound to client+user | OAuth 2.1 §4.3.1 MUST for public clients; reuse detected → revoke chain |
| Resource binding | `resource` param required, validated against AS's own base URL | RFC 8707; prevents token passthrough confusion |

### 4.4 Scope → capability mapping

OAuth `scope` is space-delimited. Maidan capabilities already use `domain:verb` — they map 1:1:

```
scope="workspace:read search:query message:post"
  → capabilities=["workspace:read", "search:query", "message:post"]
```

The consent screen renders human-readable labels from a static map (add `scope_descriptions` in the oauth module). Unknown scopes → `invalid_scope` error. The `attenuate` function already validates subset relationships — reuse it.

### 4.5 What happens to existing flows

| Existing | Change |
|---|---|
| `app_oauth.rs` (installed apps) | **Keep.** It becomes the "pre-authorized" path. Generalize its code mint/consume into shared helpers used by the new `/oauth/*` endpoints. |
| Admin token minting (`token:admin`) | **Keep.** The AS mints OAuth tokens *through* the same `create_api_token_audited` path — one token table, one audit trail. |
| OIDC relying party | **Keep.** It authenticates the *human* at the authorize endpoint. The AS doesn't replace login; it sits behind it. |
| Anonymous dev mode | **Keep.** Unauthenticated MCP stays available for dev; OAuth is for production authenticated access. |
| MCP `Authorization: Bearer <redacted>` handling | **Extend.** Add `WWW-Authenticate: Bearer <redacted> resource_metadata="..."` on 401 (currently missing per Open Work). |

---

## 5. Component Breakdown

### 5.1 New database tables

```sql
-- OAuth clients (pre-registered). CIMD clients are NOT stored (fetched live).
CREATE TABLE maidan_oauth_clients (
    id              UUID PRIMARY KEY,
    client_id       TEXT UNIQUE NOT NULL,  -- public identifier
    client_secret_hash TEXT,                -- NULL = public client
    name            TEXT NOT NULL,
    redirect_uris   TEXT[] NOT NULL,       -- exact match, except loopback
    grant_types     TEXT[] NOT NULL DEFAULT '{authorization_code,refresh_token}',
    scope           TEXT NOT NULL,         -- max requested scopes (capabilities)
    is_confidential BOOLEAN NOT NULL DEFAULT FALSE,
    created_by      UUID REFERENCES maidan_members(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ
);

-- User consent grants: "user U authorized client C for scopes S in workspace W"
CREATE TABLE maidan_oauth_consents (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    member_id       UUID NOT NULL REFERENCES maidan_members(id) ON DELETE CASCADE,
    client_id       TEXT NOT NULL,         -- pre-registered id OR CIMD URL
    scope           TEXT NOT NULL,         -- granted capabilities
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at      TIMESTAMPTZ,
    UNIQUE (workspace_id, member_id, client_id)
);

-- Refresh tokens: single-use, rotating, bound to access token lineage
CREATE TABLE maidan_oauth_refresh_tokens (
    token_hash      TEXT PRIMARY KEY,      -- SHA-256 of the plaintext
    api_token_id    UUID NOT NULL REFERENCES maidan_api_tokens(id) ON DELETE CASCADE,
    client_id       TEXT NOT NULL,
    scope           TEXT NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- rotation chain: when used, points to the successor
    replaced_by_hash TEXT REFERENCES maidan_oauth_refresh_tokens(token_hash),
    revoked_at      TIMESTAMPTZ
);
```

**Extend (don't replace):** `maidan_oauth_codes` needs `client_id`, `member_id`, `scope`, and `resource` columns. Add via migration (nullable for back-compat with app flow, required for new flow).

### 5.2 New routes (`crates/maidan-server/src/oauth/`)

| File | Endpoint | Auth | Description |
|---|---|---|---|
| `authorize.rs` | `GET /oauth/authorize` | Browser session | Validate client+redirect+scope+PKCE+resource → consent screen → mint code → 302 |
| `consent.rs` | `POST /oauth/consent` | Browser session | Record consent decision, then mint code |
| `token.rs` | `POST /oauth/token` | None (public) | `authorization_code` exchange + `refresh_token` rotation; form-encoded per RFC 6749 |
| `revoke.rs` | `POST /oauth/revoke` | Client auth or bearer | RFC 7009 revocation (access or refresh) |
| `metadata.rs` | `GET /.well-known/oauth-authorization-server` | None | RFC 8414: issuer, endpoints, PKCE, CIMD flag, scopes |
| `resource.rs` | `GET /.well-known/oauth-protected-resource` | None | RFC 9728: resource URL, authorization_servers |
| `clients.rs` | `POST/GET/DELETE /workspaces/:wid/oauth/clients` | `token:admin` | Pre-registered client CRUD |
| `cimd.rs` | (internal) | — | Fetch + validate CIMD documents (SSRF-guarded, cached) |

### 5.3 Modified files

| File | Change |
|---|---|
| `crates/maidan-server/src/error.rs` | Add `WWW-Authenticate` header on 401 for `/mcp*` (RFC 6750 + RFC 9728 discovery) |
| `crates/maidan-server/src/app_oauth.rs` | Extract code mint/consume/PKCE into shared `oauth::codes` helpers |
| `crates/maidan-types/src/models.rs` | `OAuthClient`, `NewOAuthClient`, `OAuthConsent`, `OAuthRefreshToken` types |
| `crates/maidan-store/src/store.rs` | `OAuthClientStore`, `OAuthConsentStore`, `OAuthRefreshTokenStore` traits |
| `migrations/{pg,sqlite}/` | 3 new tables + `maidan_oauth_codes` extension |
| `crates/maidan-server/src/openapi/` | Document new endpoints (utoipa) |
| `docs/Clients.md` | Dev-instance recipe already names `MAIDAN_ALLOWED_HOSTS`; add OAuth client setup |

---

## 6. Token Lifetimes

| Token | Lifetime | Rationale |
|---|---|---|
| Authorization code | 10 min, single-use | Matches existing `CODE_TTL_SECS`; short enough to limit replay window |
| Access token | 1 hour | Standard; short enough that revocation latency is bounded; MCP calls are frequent |
| Refresh token | 30 days, rotating | Long-lived UX without long-lived secrets; rotation detects theft (reuse → revoke chain) |
| Consent grant | Until revoked | Like app installations; user manages from console |
| Client registration | Until revoked | Admin-managed |

**Refresh rotation details:** Each use consumes the old refresh token and issues a new pair. If a consumed refresh token is presented again → suspected theft → revoke the entire chain (all tokens derived from that grant) and audit. This is the OAuth 2.1 §4.3.1 requirement.

---

## 7. Security Considerations

1. **No second ACL.** Capabilities are the scope vocabulary. The `is_delegatable` filter applies — OAuth can never grant `token:admin` or cross-tenant caps.
2. **PKCE required for public clients.** S256 only (plain rejected). New for these flows: the installed-app exchange in `app_oauth.rs` makes PKCE optional.
3. **Redirect URI validation.** Exact match for confidential clients; loopback (`http://127.0.0.1:*`, `http://localhost:*`) allows any port per RFC 8252 §7.3 (CLI client requirement). HTTPS required for non-loopback.
4. **CIMD fetching is SSRF-guarded.** Allowlist schemes (https only), no private IPs, response size cap, cache with TTL, `client_id` must equal the fetch URL.
5. **Resource binding.** `resource` param must equal the AS's canonical base URL. Tokens are bound to the issuing Maidan instance — never accepted from another issuer.
6. **Consent is per-user, not per-admin.** Unlike the app flow (admin authorizes), OAuth authorize requires the *resource owner's* session. An admin can't consent on behalf of users.
7. **Audit everything.** Code mint, code exchange, refresh, revocation, consent grant/revoke — all `*_audited` with actor, client, scope.
8. **Rate limiting.** Token endpoint gets strict rate limits (it's unauthenticated). Failed PKCE/exchange attempts are logged.
9. **No token passthrough.** The MCP server never forwards the client's bearer upstream (already the rule; OAuth doesn't change it).

---

## 8. Local Dev Tooling (What to Add)

| Tool | Purpose | How |
|---|---|---|
| **Test OAuth client (CLI)** | Exercise authorize/token/refresh flows without a browser | New `maidan` CLI subcommand or a small Rust example using the `oauth2` crate (client-side) against local server |
| **CIMD test document** | Serve a fake CIMD for local testing | Static JSON file served by dev server or `python3 -m http.server`; `client_id` = local URL |
| **Token inspector** | Decode/inspect opaque tokens, check capabilities | Extend `maidan-cli` with `token inspect <token>` (hash → DB lookup → show member, caps, expiry) |
| **OAuth flow E2E test** | Scripted authorize→consent→token→refresh→revoke | Integration test in `crates/maidan-server/tests/` using axum test client + mock browser session |
| **MCP Inspector with OAuth** | Validate the full MCP+OAuth handshake | The official `@modelcontextprotocol/inspector` already supports OAuth; document the local recipe |
| **Well-known validators** | Check RFC 8414/9728 metadata correctness | `scripts/check-oauth-metadata.sh` — curls both endpoints, validates JSON schema |
| **Consent screen** | Minimal HTML form (no framework) | Server-rendered in `consent.rs`; styled to match `/ui` minimally |

**Dev setup additions** (`.env` / compose):
```bash
# Enable the OAuth AS (off by default; never on in the insecure quickstart)
MAIDAN_OAUTH_ENABLED=1
MAIDAN_OAUTH_ISSUER=https://maidan.example.com  # canonical base URL
# For local testing with the MCP Inspector:
MAIDAN_OAUTH_ALLOW_HTTP_REDIRECTS=1  # dev only, loopback
```

---

## 9. Implementation Plan with Sizes

| Phase | Work | Size | Depends on |
|---|---|---|---|
| **P0: Resource server compliance** | `WWW-Authenticate` on MCP 401s; `/.well-known/oauth-protected-resource` | XS (1–2 days) | Nothing — do first, unblocks discovery |
| **P1: AS metadata** | `/.well-known/oauth-authorization-server` (RFC 8414) | XS (1 day) | P0 |
| **P2: Client registry** | `maidan_oauth_clients` table + CRUD routes + CIMD fetcher | S (3–4 days) | P1 |
| **P3: Authorize + consent** | `GET /oauth/authorize`, consent screen, code minting (generalize `app_oauth.rs`) | M (1 week) | P2, OIDC session |
| **P4: Token endpoint** | `POST /oauth/token` (code exchange + refresh rotation), `POST /oauth/revoke` | M (1 week) | P3 |
| **P5: Consent management** | `maidan_oauth_consents` table, console UI for grant/revoke | S (3 days) | P3 |
| **P6: Hardening** | Rate limits, SSRF guards, audit coverage, scope contract tests | S (3–4 days) | P4 |
| **P7: Validation record** | E2E against MCP clients with real OAuth (feeds the validation record before any listing) | S (2–3 days) | P6, public instance |

**Total: ~5–6 weeks** for a production-ready OAuth 2.1 AS, phased so P0–P1 ship independently.

**What "making it real" requires beyond code:**
1. **Migrations:** 3 new tables + `maidan_oauth_codes` extension (both pg + sqlite).
2. **Config:** `MAIDAN_OAUTH_ENABLED`, `MAIDAN_OAUTH_ISSUER`, redirect allowlists. Off by default.
3. **Docs:** OAuth client developer guide; consent UX copy; updated `docs/Clients.md`.
4. **Directory submissions:** Connected-app directory listings require OAuth — this unblocks the resubmit path if API-key auth is rejected.
5. **Operational:** Token endpoint monitoring/alerting; refresh-token chain revocation runbook; CIMD cache invalidation.

---

## 10. Open questions for the maintainer, decided 2026-10-08

1. **Scope of v1:** **A — Full OAuth 2.1, all 7 phases** (~5–6 weeks), landing P0–P1 as an early reviewable milestone.
2. **Consent UX:** **B — Integrate into the existing `/ui` console.** The console is the trust root; no second surface.
3. **Client secrets:** **B — Both public and confidential clients from day one.** (Research had recommended public-only first; the maintainer chose otherwise — build both.)
4. **DCR:** **A — CIMD + pre-registered only, skip DCR.** Spec deprecates it.

---

## Appendix: Key Files Reference

- `crates/maidan-server/src/app_oauth.rs` — existing code flow to generalize
- `crates/maidan-server/src/oidc/handlers.rs` — session auth pattern for authorize endpoint
- `crates/maidan-server/src/routes/token.rs` — mint/attenuate/revoke patterns
- `crates/maidan-auth/src/capability.rs` — scope vocabulary
- `crates/maidan-auth/src/context.rs` — `AuthContext` constructors
- `migrations/postgres/0029_oauth_codes.sql` — existing codes table
- `docs/Protocols.md` — J6 (MCP OAuth), protocol inventory
