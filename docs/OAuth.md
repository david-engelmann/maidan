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

Phases zero to three have shipped. Each later phase is its own PR, held to
the security controls in Next 23.

| Phase | What ships | Acceptance criteria |
|---|---|---|
| P0, resource metadata | RFC 9728 metadata for the MCP endpoint at `/.well-known/oauth-protected-resource/mcp/streamable`, and a `WWW-Authenticate` challenge on a 401 from that endpoint, both off until `MAIDAN_PUBLIC_ORIGIN` is set | An unauthenticated `POST /mcp/streamable` answers 401 with `Bearer resource_metadata="<origin>/.well-known/oauth-protected-resource/mcp/streamable"`. `/mcp` and other 401s carry no challenge, and no root-form document is served, since a client must reject metadata whose `resource` is not the URL it asked. The document names no authorization server yet (`oauth_resource_metadata_e2e`) |
| P1, client registry | Pre-registered clients and client metadata documents | Metadata documents are fetched only through the egress guard. A client id resolves to exactly one client |
| P2, authorize and consent | `GET /oauth/authorize` and the consent page in `/ui` | PKCE S256 only. Redirect URIs match exactly. The consent page cannot be framed. A grant never exceeds the member's capabilities, and a scope names only a delegatable capability, never `token:*`, `operator:*` or `audit:read-global` |
| P3, token endpoint and server metadata | `POST /oauth/token`, `POST /oauth/revoke`, and the RFC 8414 document, which lists only the endpoints and grants that now exist | A code is consumed only after its client, `redirect_uri`, verifier and resource all validate, in one atomic step, so a stolen code cannot be burned by a wrong guess. Every access and refresh token from one grant carries the grant's lineage id, and a reused refresh token revokes everything under that id. Tokens are bound to the MCP resource (RFC 8707). Responses carry `iss` (RFC 9207). The resource document gains `authorization_servers` |
| P4, consent management | Listing and revoking grants in the console | A revoked grant stops working on the next request |
| P5, hardening | Rate limits on the new endpoints, audit rows, scope contract tests | A contract fails on a grant wider than its member |
| P6, validation record | The full flow from real MCP clients against a public instance | The record names the commit it ran |

The authorization-server document waited for P3 because RFC 8414 requires
`response_types_supported` and an authorization endpoint, and before P3 any
such document would either be invalid or advertise endpoints that do not exist.

## Phase three controls

P3 shipped. These are its controls and what each one does.

- **Server off without an origin.** With no `MAIDAN_PUBLIC_ORIGIN`,
  `/oauth/authorize`, `/oauth/token` and `/oauth/revoke` answer 404, like the
  discovery documents. The token and revocation endpoints check it in a layer,
  before the body is read.
- **Resource binding (RFC 8707).** Authorize and token requests must name
  `resource`, and it must be `<MAIDAN_PUBLIC_ORIGIN>/mcp/streamable`. The code,
  the refresh token and the minted access token all carry it. The bearer
  middleware accepts an access token minted under an OAuth grant only when the
  matched route is `/mcp/streamable`. Everywhere else, `/mcp` included, the
  token resolves as no token at all and the answer is 401 with
  `WWW-Authenticate: Bearer error="invalid_token"`. The refusal lives in
  `maidan_auth::resolve_bearer`, so the WebSocket, gRPC and session-from-token
  paths refuse it too. Only `resolve_resource_bearer` accepts it.
- **Client authentication.** `client_secret_basic`, `client_secret_post` and
  `none`, at the token and revocation endpoints. Two methods in one request is
  `invalid_request`. A confidential client that sends no secret, or a wrong
  one, is `invalid_client`, with `WWW-Authenticate: Basic realm="maidan"` when
  it tried Basic. A public client may not send a secret. Secrets are compared
  in constant time.
- **Code exchange.** The code is read and fully validated (client, redirect
  URI, resource, PKCE verifier) before one atomic statement consumes it. The
  exchange returns an access token (one hour) and a refresh token (30 days),
  minted with their audit row in one transaction.
- **Refresh rotation.** One transaction marks the presented refresh token used,
  only if it is unused, unrevoked, unexpired and its grant is live, and mints
  the new access token and refresh token under the same grant and lineage. The
  new access token's member and workspace come from the grant, never from the
  request. `scope` may narrow the grant's scope and never widen it
  (`invalid_scope`). Omitted, it is the grant's scope.
- **Reuse revokes the lineage.** A refresh token presented after it was used,
  including the loser of two concurrent rotations, revokes every grant with
  its lineage id, every access token under those grants and every refresh
  token of the lineage, in one transaction with an `oauth_lineage.revoke`
  audit row (reason `refresh_token_reuse`). A code presented again after it
  was used, with its PKCE verifier, does the same for the live grant matching
  the code's client, member, workspace and scope (reason `code_reuse`). The
  code without its verifier is refused and revokes nothing, since anyone who
  saw the redirect holds it.
- **A revoked grant stops its tokens and codes.** The bearer lookup refuses a
  token whose grant is revoked, and a lineage revocation also revokes the
  token rows themselves, so the next request fails either way. A code whose
  grant was revoked before the exchange is `invalid_grant`: the exchange never
  starts a new grant, so a revocation cannot be outrun by a code in flight.
- **Revocation (RFC 7009).** `POST /oauth/revoke` takes `token` and an
  optional `token_type_hint`. A refresh token revokes its lineage (reason
  `client_revocation`), an access token revokes itself (`oauth_token.revoke`).
  It answers 200 with an empty body for a known token, an unknown one and
  another client's alike, and leaves another client's token untouched.
- **A narrowed client stops refreshing.** When an operator narrows a client's
  allowed scopes, a code exchange or refresh that would carry a scope the
  client no longer allows is `unauthorized_client`.
- **Rate limits.** The token, revocation, authorize and consent endpoints
  are charged under two buckets of their own, tighter than the API at
  large. One bucket is per client id (120 requests a minute by default),
  the other per address across all client ids (600 a minute). The client
  id is read from the Basic username, the authorize query, or a small
  form body, and it names a bucket only; it is unauthenticated at that
  layer, which is why the address bucket sits beside it. A limited
  request answers 429 with `Retry-After`. Operators tune or disable the
  buckets with `MAIDAN_OAUTH_CLIENT_RATE_LIMIT_MAX`,
  `MAIDAN_OAUTH_CLIENT_RATE_LIMIT_WINDOW_SECS`,
  `MAIDAN_OAUTH_IP_RATE_LIMIT_MAX` and
  `MAIDAN_OAUTH_IP_RATE_LIMIT_WINDOW_SECS`.
- **RFC 6749 errors.** The token and revocation endpoints answer client errors
  as `{"error", "error_description"}` with `Cache-Control: no-store`, not as
  problems, because OAuth clients parse that envelope. A body over the size
  limit, a 404 and a 429 stay problems: they come from layers in front of the
  handler.

## Configuration

`MAIDAN_PUBLIC_ORIGIN` is the instance's public origin, for example
`https://maidan.example.com`: `https` and a host with no path (`http` only on a
loopback host). OAuth identifiers are built from it, never from a request's
`Host`, so a forged header cannot make the server name another origin. A
malformed value refuses boot.

## Code layout

New code lives in `crates/maidan-server/src/oauth/`, apart from the
installed-app flow in `app_oauth.rs`. Migrations for this work use 0150 to 0159.
