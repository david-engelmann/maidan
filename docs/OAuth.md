# OAuth authorization server

Maidan is becoming an OAuth 2.1 authorization server (Open Work Next 23,
decided 2026-10-08), so MCP clients and connected-app directories can sign
people in against it. The research behind this plan is archived under
`docs/archive/`.

## Decisions (2026-10-08)

- **Build it, rather than adopt a crate.** Tokens are capability-scoped, which
  a generic crate does not model. What exists today is the installed-app
  one-time code exchange with optional PKCE (see Claims).
- **Full OAuth 2.1 scope, in seven phases.**
- **Consent happens in the `/ui` console.**
- **Public and confidential clients** from the start.
- **No dynamic client registration, by choice.** Clients register through
  client metadata documents or pre-registration.
- **Capabilities are the scopes**, never wider than the member's own.
- **An OAuth token never accepts an approval gate.** Only a signed-in browser
  session or a token holding `approval:grant` can (#1325).

## Phases

Phase zero is on `main`. Each later phase is its own PR, held to the security
controls in Next 23.

| Phase | What ships | Acceptance criteria |
|---|---|---|
| P0, resource metadata | RFC 9728 metadata for the MCP endpoint at `/.well-known/oauth-protected-resource/mcp/streamable` (and the root form), and a `WWW-Authenticate` challenge on a 401 from an MCP route, both off until `MAIDAN_PUBLIC_ORIGIN` is set | An unauthenticated `POST /mcp/streamable` answers 401 with `Bearer resource_metadata="<origin>/.well-known/oauth-protected-resource/mcp/streamable"`. Other 401s carry no challenge. The document names no authorization server yet (`oauth_resource_metadata_e2e`) |
| P1, client registry | Pre-registered clients and client metadata documents | Metadata documents are fetched only through the egress guard. A client id resolves to exactly one client |
| P2, authorize and consent | `GET /oauth/authorize` and the consent page in `/ui` | PKCE S256 only. Redirect URIs match exactly. The consent page cannot be framed. A grant never exceeds the member's capabilities |
| P3, token endpoint and server metadata | `POST /oauth/token`, `POST /oauth/revoke`, and the RFC 8414 document, which lists only the endpoints and grants that now exist | Refresh tokens rotate and a reused one revokes its family. Tokens are bound to the MCP resource (RFC 8707). Responses carry `iss` (RFC 9207). The resource document gains `authorization_servers` |
| P4, consent management | Listing and revoking grants in the console | A revoked grant stops working on the next request |
| P5, hardening | Rate limits on the new endpoints, audit rows, scope contract tests | A contract fails on a grant wider than its member |
| P6, validation record | The full flow from real MCP clients against a public instance | The record names the commit it ran |

The authorization-server document waits for P3 because RFC 8414 requires
`response_types_supported` and an authorization endpoint, and before P3 any
such document would either be invalid or advertise endpoints that do not exist.

## Configuration

`MAIDAN_PUBLIC_ORIGIN` is the instance's public origin, for example
`https://maidan.example.com`: `https` and a host with no path (`http` only on a
loopback host). OAuth identifiers are built from it, never from a request's
`Host`, so a forged header cannot make the server name another origin. A
malformed value refuses boot.

## Code layout

New code lives in `crates/maidan-server/src/oauth/`, apart from the
installed-app flow in `app_oauth.rs`. Migrations for this work use 0150 to 0159.
