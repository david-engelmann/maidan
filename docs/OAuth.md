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
  client metadata documents or pre-registration. *Superseded on 2026-11-05;
  see below.*
- **Capabilities are the scopes**, never wider than the member's own.
- **An OAuth token never accepts an approval gate.** Only a signed-in browser
  session or a token holding `approval:grant` can (#1325).

## Decision (2026-11-05): every MCP client signs in with OAuth

The maintainer reversed the 2026-10-08 "no dynamic registration" point
([Decisions](Decisions.md)), knowing it opens client registration to anyone:

- **RFC 7591 dynamic client registration** for public clients only, behind
  `MAIDAN_OAUTH_DYNAMIC_REGISTRATION`.
- **RFC 8252 loopback redirects whose port may vary.** Exact matching stays
  for every other redirect.

A person pastes `https://maidan.world/mcp/streamable` into any MCP client,
signs in in the browser, and is done; no token is copied.

## Phases

Phases zero to three have shipped. Each later phase is its own PR, held to
the security controls in Next 23.

| Phase | What ships | Acceptance criteria |
|---|---|---|
| P0, resource metadata | RFC 9728 metadata for the MCP endpoint at `/.well-known/oauth-protected-resource/mcp/streamable`, and a `WWW-Authenticate` challenge on a 401 from that endpoint, both off until `MAIDAN_PUBLIC_ORIGIN` is set | An unauthenticated `POST /mcp/streamable` answers 401 with `Bearer resource_metadata="<origin>/.well-known/oauth-protected-resource/mcp/streamable"`. `/mcp` and other 401s carry no challenge, and no root-form document is served, since a client must reject metadata whose `resource` is not the URL it asked. The document names no authorization server yet (`oauth_resource_metadata_e2e`) |
| P1, client registry | Pre-registered clients and client metadata documents | Metadata documents are fetched only from hosts the operator lists in `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`. A client id resolves to exactly one client |
| P2, authorize and consent | `GET /oauth/authorize` and the consent page in `/ui` | PKCE S256 only. Redirect URIs match exactly, except that an `http` loopback redirect's port may vary (RFC 8252, since 2026-11-05). The consent page cannot be framed. A grant never exceeds the member's capabilities, and a scope names only a delegatable capability, never `token:*`, `operator:*` or `audit:read-global` |
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
- **Client metadata hosts.** A client whose `client_id` is an HTTPS URL is
  resolved from the document at that URL, but only when the URL's host is
  listed exactly in `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS` (comma-separated
  host names, empty by default) and names no port. Any other host is refused
  with 403 before anything is fetched, and redirects are never followed. The
  guard is against server-side request forgery, which is the operator's
  concern, so it is an instance setting and not a workspace one: a workspace
  fresh from sign-up resolves the same clients as every other. The hosted
  instances list `claude.ai` (Claude, Claude Code), `chatgpt.com` (ChatGPT,
  Codex), `zed.dev` and `vscode.dev`. A client that sees
  `client_id_metadata_document_supported` uses its document and does not fall
  back to registering, so leaving its host out breaks it.
- **Signing in from the authorization endpoint.** A browser that reaches
  `GET /oauth/authorize` with no credential at all, usually a person arriving
  from an MCP client before signing in, is answered 302 to
  `/auth/oidc/login?return_to=<the authorize path and query>`. After sign-in
  it comes back to the same request and goes on to consent. `return_to` is
  honoured only as a same-origin path: `//host`, an absolute URL, a backslash
  or a control character falls back to `/ui/`. A request that carries an
  `Authorization` header, valid or not, is never redirected, and a bad bearer
  still answers 401. With no sign-in configured the endpoint answers 401.
- **Rate limits.** The token, revocation, authorize and consent endpoints
  are charged under two buckets of their own, tighter than the API at
  large. One bucket is per client id and address pair (120 requests a
  minute by default), the other per address across all client ids (600 a
  minute). The client id is read from the Basic username, the authorize
  query, or a small form body. It is unauthenticated at that layer, which
  is why the client bucket is keyed by the pair; a bucket named by the id
  alone would let any address spend a named client's budget and lock
  that client out. A limited request answers 429 with `Retry-After`.
  Operators tune or disable the buckets with
  `MAIDAN_OAUTH_CLIENT_RATE_LIMIT_MAX`,
  `MAIDAN_OAUTH_CLIENT_RATE_LIMIT_WINDOW_SECS`,
  `MAIDAN_OAUTH_IP_RATE_LIMIT_MAX` and
  `MAIDAN_OAUTH_IP_RATE_LIMIT_WINDOW_SECS`.
- **RFC 6749 errors.** The token and revocation endpoints answer client errors
  as `{"error", "error_description"}` with `Cache-Control: no-store`, not as
  problems, because OAuth clients parse that envelope. A body over the size
  limit, a 404 and a 429 stay problems: they come from layers in front of the
  handler.

## Dynamic client registration

`POST /oauth/register` (RFC 7591) lets an MCP client that has neither a
pre-registration nor a client metadata document the instance fetches register
itself: Cursor, Gemini CLI, the MCP Inspector, the SDK examples. It is off
unless `MAIDAN_OAUTH_DYNAMIC_REGISTRATION=1`, and only then does the RFC 8414
document name `registration_endpoint`. Turning it off answers 404 and refuses
every client that registered itself, at authorize and at the token endpoint.

- **Public clients only.** `token_endpoint_auth_method` must be `none` or
  omitted (an omitted one is answered as `none`); no secret is ever issued,
  and PKCE S256 is required at authorize as for every client. `grant_types`
  may name only `authorization_code` and `refresh_token`, `response_types`
  only `code`.
- **Redirects that land on the person's own machine.**
  - `http` on `127.0.0.1`, `[::1]` or `localhost`, exactly (not the rest of
    `127/8`, not a name that resolves to loopback). At authorize the port may
    differ from the registered one, per RFC 8252 section 7.3; scheme, host,
    path and query must match, and `localhost` never stands in for an IP
    literal.
  - A private-use scheme in reverse-DNS form (`com.example.app:`) or a known
    MCP client's (`cursor`, `vscode`, `vscode-insiders`, `windsurf`, `zed`).
  - `https` only on a host in `MAIDAN_OAUTH_DCR_REDIRECT_HOSTS`. maidan.world
    lists `vscode.dev` and `insiders.vscode.dev`, for VS Code's
    `https://vscode.dev/redirect`.
  - `javascript:`, `data:`, `file:` and other script or local schemes, `http`
    to any other host, a fragment or credentials refuse the registration
    (`invalid_redirect_uri`). An `https` redirect on an unlisted host is left
    out of the registration rather than refusing it, because Cursor registers
    `https://www.cursor.com/agents/mcp/oauth/callback` beside the loopback
    callback it uses; the response lists what was registered.
- **What a client may ask for.** A `scope` naming delegatable capabilities,
  or none, which allows any delegatable capability. What a token carries is
  still cut at authorize to what the consenting member holds, through the
  same grant path as every client, and never includes `approval:grant`.
- **Its name is unverified.** `client_name` is at most 100 characters with no
  control or invisible formatting characters (bidirectional overrides,
  zero-width joiners). The consent page shows it as "Cursor (unverified app,
  registered by itself)", says where the browser goes next (an app on this
  computer and its port, the app that opens a scheme, or the `https` host),
  and warns to allow only a sign-in the person just started. `client_uri`,
  `logo_uri` and other metadata are ignored, so nothing else a client says
  reaches the page.
- **Bounded.** At most ten redirect URIs of 512 bytes each and a 16 KiB body.
  Registration is charged per address (20 an hour by default,
  `MAIDAN_OAUTH_REGISTER_RATE_LIMIT_MAX` and `_WINDOW_SECS`), and at most
  10,000 registrations may wait on a first sign-in at once; past that,
  registration answers 429 until room is made.
- **Swept.** A client that completes no token exchange within 24 hours of
  registering, or no exchange or refresh for 90 days, is deleted hourly with
  its grants, codes and tokens (migration 0220 on Postgres, 0219 on SQLite).
  The sweep runs whether or not registration is on.

### Red team (2026-11-05)

- **Consent phishing and open redirect.** An attacker can register a client
  named anything and send a person an authorize link. The code goes only to a
  loopback address or an app scheme on the victim's own machine, or to a host
  the operator listed, never to the attacker's server, and the page marks the
  name unverified and names the destination. An `https` redirect is never
  registered unless its host is listed, and authorize refuses any redirect the
  client did not register.
- **Code interception on loopback.** Another process on the person's machine
  that answers the loopback port gets the code but not the PKCE verifier,
  which the real client holds, so it cannot redeem it. A malicious local app
  that starts its own flow is consent phishing, above.
- **Registration flooding.** Charged per address, capped globally at 10,000
  rows awaiting a first sign-in, swept after a day. A distributed flood can
  fill the cap and hold new registrations off for up to a day, which refuses
  clients rather than filling the table.
- **A client calling itself "Claude".** Its name is shown unverified, with the
  warning. Claude and Claude Code identify themselves by their metadata
  documents on claude.ai, which a registration cannot spoof (a registered
  `client_id` is server-chosen, `dyn-<uuid>`, never a URL).
- **Redirect matching.** Port leeway applies only to `http` on the three
  loopback hosts; `127.0.0.2`, `localhost.evil.example`, an added path,
  query, trailing slash, credentials or fragment all miss
  (`registry::redirect_uri_matches`).

## P6 validation record

`scripts/oauth-p6-record.py` runs the flow a remote MCP client runs and writes
a JSON record of each step, named by the commit it ran against:

1. Discovery. An unauthenticated `POST /mcp/streamable` must answer 401 with a
   `resource_metadata` challenge. The resource document must name
   `<origin>/mcp/streamable` and an authorization server, and the RFC 8414
   document must offer PKCE S256 only and client metadata documents, and a
   registration endpoint, if it names one, must be `<issuer>/oauth/register`.
2. Consent. The script opens the authorize URL and waits up to ten minutes on
   a loopback redirect. The redirect must carry the same `state` and an `iss`
   equal to the issuer (RFC 9207).
3. Token. The code is exchanged with the PKCE verifier and the resource
   (RFC 8707). The response must be `no-store` and carry a refresh token.
4. `tools/list`. The access token must initialize `/mcp/streamable` and list
   at least one tool, and `/mcp` must refuse it with 401.

The script then revokes the refresh token. The record holds tokens only as
SHA-256 prefixes. Run it from a machine with a browser signed in to the
instance:

```sh
python3 scripts/oauth-p6-record.py --origin https://<instance> \
  --client-id https://<client-metadata-host>/client.json
```

`--commit` names the commit when the instance's `/health` version has no
`v<version>` tag. The script exits non-zero when any check fails, and still
writes the record. `scripts/test-oauth-p6-record.py` runs the checks against
the documents in `scripts/fixtures/oauth-p6`, in the coverage job.

## Configuration

`MAIDAN_PUBLIC_ORIGIN` is the instance's public origin, for example
`https://maidan.example.com`: `https` and a host with no path (`http` only on a
loopback host). OAuth identifiers are built from it, never from a request's
`Host`, so a forged header cannot make the server name another origin. A
malformed value refuses boot.

## Code layout

New code lives in `crates/maidan-server/src/oauth/`, apart from the
installed-app flow in `app_oauth.rs`. Migrations for this work use 0150 to 0159,
plus 0172 (client metadata rows) and 0220/0219 (self-registered clients).
