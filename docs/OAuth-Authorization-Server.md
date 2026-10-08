# OAuth authorization server: research, architecture and build plan

Status: research and design. Nothing on this page is built. It answers the
maintainer's decision of 2026-10-08 on Lane 8, the OAuth authorization server
that the connected-apps directory lanes wait on: research the best solution,
recommend the architecture, add the tools the local setup needs, and write
down what building it takes. The same decision round made `approval_decide` a
model-callable tool, so this design also covers how an approval stays a human's
decision when the caller holds an OAuth token. Where this page and
[Decisions](Decisions.md) or [Open Work](Open%20Work.md) disagree, those pages
win until the maintainer accepts this one.

Facts about other projects were checked on 2026-10-08 against the sources at
the end. Specifications and directory rules move, so each build PR re-checks
the rows it relies on.

## The recommendation in brief

Maidan becomes its own OAuth 2.1 authorization server, on the same origin as
its MCP endpoints, and keeps the people-facing sign-in where it already is: the
operator's OIDC provider through Maidan's existing relying-party flow, or a
pasted member token where there is no provider. The server issues Maidan's own
opaque `maid_` tokens, hashed and looked up on every request like every other
Maidan token, each tied to a grant row so that revocation is immediate. Client
ID Metadata Documents are the main way clients identify themselves, with
dynamic registration kept for the clients that still need it. A grant binds
one workspace and one resource, and its scopes are capability strings from the
delegatable work set, intersected with what the person consents to and a
per-workspace ceiling. Authority capabilities, `approval:grant` among them, are
never available through OAuth. Recording an approval through `approval_decide`
needs a per-decision step-up that sends the person through Maidan's own page,
which shows the evidence and re-checks who they are.

Delegating everything to the operator's identity provider and running a
separate authorization server beside Maidan were both examined and rejected.
The reasons are in [Options compared](#options-compared).

## What MCP clients require

MCP authorization is optional, but an HTTP server that requires authentication
is expected to follow it, and every directory below that lists authenticated
servers expects it. The requirements grew across four revisions.

| Revision | What it asks of a server and its authorization server |
| --- | --- |
| [2025-03-26](https://modelcontextprotocol.io/specification/2025-03-26/basic/authorization) | Authorization server metadata (RFC 8414) at the MCP server's origin, with fallback default paths `/authorize`, `/token` and `/register` when there is none. Dynamic client registration (RFC 7591) is a SHOULD. PKCE required |
| [2025-06-18](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization) | The MCP server is a resource server and MUST publish Protected Resource Metadata (RFC 9728) naming its authorization servers. Clients MUST send resource indicators (RFC 8707), and servers MUST check that a token was issued for them |
| [2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization) | Client ID Metadata Documents become the preferred registration (SHOULD), dynamic registration drops to MAY, OpenID Connect discovery is accepted, and runtime scope challenges with step-up are described |
| [2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization) | The current revision, detailed below. Dynamic registration is described as deprecated and kept for backward compatibility. RFC 9207 issuer identification is a SHOULD with a stated plan to make it a MUST |

What 2026-07-28 requires, with the parts Maidan's server has to answer:

- **Discovery.** The MCP server MUST publish Protected Resource Metadata with
  `authorization_servers`. A client finds it through the `resource_metadata`
  parameter of `WWW-Authenticate` on a 401, or by trying the well-known URI with
  the endpoint path inserted (`/.well-known/oauth-protected-resource/mcp`) and
  then at the root
  ([discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery)).
  The metadata's `resource` must be identical to the URL the client used
  ([RFC 9728](https://www.rfc-editor.org/rfc/rfc9728)).
- **Authorization server metadata.** RFC 8414 or OpenID Connect discovery. The
  client tries the OAuth path-inserted URI, then the OpenID path-inserted URI,
  then the OpenID path-appended URI, and the `issuer` in the document must equal
  the issuer it expected exactly ([RFC 8414](https://www.rfc-editor.org/rfc/rfc8414)).
- **PKCE.** Clients MUST check `code_challenge_methods_supported` and refuse to
  proceed when it is absent, and use `S256` when they can
  ([RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)).
- **Resource indicators and audience.** Clients MUST send `resource`, the
  canonical server URI without a trailing slash, on both the authorization and
  the token request, whether or not the server supports it. Servers MUST
  validate that a token was issued for them, MUST NOT accept or pass on any
  other token, and answer an invalid or expired token with 401
  ([RFC 8707](https://www.rfc-editor.org/rfc/rfc8707)).
- **Issuer in the response.** Authorization servers SHOULD return `iss` with
  the code and advertise `authorization_response_iss_parameter_supported`;
  clients compare it to the recorded issuer before sending the code anywhere
  ([RFC 9207](https://www.rfc-editor.org/rfc/rfc9207)).
- **Registration order.** A pre-registered client first, then a Client ID
  Metadata Document when the server advertises
  `client_id_metadata_document_supported`, then dynamic registration when there
  is a `registration_endpoint`, then asking the user
  ([client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)).
- **Scopes.** A 401 SHOULD carry the scopes the request needs; with none,
  clients request `scopes_supported`, which is meant to be the minimal set. A
  token with too little scope gets 403 with `error="insufficient_scope"`, the
  needed scopes and `resource_metadata`, all in one challenge. Clients union
  the new scopes with the ones they had and re-authorize. Servers MUST account
  for scope hierarchies, and need not list every dynamically issued scope in
  `scopes_supported` ([RFC 6750](https://www.rfc-editor.org/rfc/rfc6750)).
- **Refresh tokens.** Servers SHOULD NOT put `offline_access` in their scope
  lists. OAuth 2.1 requires refresh tokens for public clients to be rotated or
  sender-constrained ([OAuth 2.1](https://datatracker.ietf.org/doc/draft-ietf-oauth-v2-1/)).
- **Security considerations.** Exact redirect URI matching, HTTPS for the
  authorization server, loopback or HTTPS redirects only, mix-up and confused
  deputy defenses, and for metadata documents protection against server-side
  request forgery, warnings for loopback redirect URIs, showing the redirect
  host on consent, and a trust policy
  ([security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations)).
- **Extensions.** Enterprise-Managed Authorization (an identity assertion
  grant from the organization's provider) is an optional extension
  ([extension](https://modelcontextprotocol.io/extensions/auth/enterprise-managed-authorization)).
  It needs an authorization server that Maidan controls, which the
  recommendation provides, and is left for later.

The Client ID Metadata Document draft
([draft-ietf-oauth-client-id-metadata-document](https://datatracker.ietf.org/doc/draft-ietf-oauth-client-id-metadata-document/),
revision 02 of July 2026) adds the rules a server must follow when it fetches
one: a 200 response only, no redirects followed, no fetches to special-use
addresses outside a development loopback exception, a read limit of about 5 KB,
no caching of errors or invalid documents, and no client secrets in the
document (private key JWT is allowed).

## What each directory and client needs

| Surface | Registration it uses | Redirect URIs | What else it requires |
| --- | --- | --- | --- |
| ChatGPT apps and the app directory ([auth](https://developers.openai.com/apps-sdk/build/auth), [submission](https://developers.openai.com/apps-sdk/app-submission-guidelines)) | Metadata document preferred, at `https://chatgpt.com/oauth/client.json` or a per-callback document; dynamic registration or a predefined client otherwise | `https://chatgpt.com/connector_platform_oauth_redirect` when RFC 9207 is met, otherwise a per-callback URI | Protected Resource Metadata, authorization server metadata, `S256` advertised, the `resource` echoed into the token's audience. Its document lists `token_endpoint_auth_methods_supported` as a list (`none`, `private_key_jwt`) alongside the singular field. Per-tool `securitySchemes` and a `mcp/www_authenticate` tool result trigger linking. Review needs a working demo account reachable without sign-up or second-factor steps, and domain verification |
| Claude connectors and directory ([authentication](https://claude.com/docs/connectors/building/authentication)) | Metadata document only when the metadata says `client_id_metadata_document_supported` and lists `none`; otherwise dynamic registration, which registers a new client on every fresh connection. Static headers are a limited beta | `https://claude.ai/api/mcp/auth_callback`; Claude Code uses port-agnostic `http://localhost/callback` and `http://127.0.0.1/callback` | A 401 (a challenge on a 200 is ignored). Only the first authorization server listed is used. 10 second timeouts on discovery, registration and token calls. Refresh failures must be `invalid_grant`, and refresh tokens rotate. The resource in the metadata must match the URL exactly. A URL-pattern listing fits per-customer URLs, which suits self-hosting. Review needs test credentials for a populated account. Its egress range is `160.79.104.0/21` |
| Cursor and its Marketplace ([MCP](https://cursor.com/docs/mcp), [plugins](https://cursor.com/docs/plugins)) | Dynamic registration, or a static client id and secret in the server's `auth` block. Its docs do not mention metadata documents | `https://www.cursor.com/agents/mcp/oauth/callback` and `http://localhost:8787/callback` | Marketplace listings are manually reviewed and open source |
| VS Code ([MCP guide](https://code.visualstudio.com/api/extension-guides/ai/mcp), [Keycloak guide](https://www.keycloak.org/securing-apps/mcp-authz-server)) | Dynamic registration, then a static client id the user enters. The Keycloak guide also configures VS Code's metadata document at `https://vscode.dev/oauth/client-metadata.json` | `http://127.0.0.1:33418` and `https://vscode.dev/redirect` | Sends `resource` |
| Gemini CLI ([MCP servers](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md)) | Discovery from a 401, then dynamic registration when offered | `http://localhost:<random port>/oauth/callback` | Validates RFC 9207 `iss` |
| MCP Registry and GitHub's listing ([registry](https://github.com/modelcontextprotocol/registry)) | None. The registry holds metadata (`server.json` with `remotes` and header declarations) | None | A remote must be publicly reachable. No OAuth requirement |
| Smithery ([publish](https://smithery.ai/docs/build/publish)) | Not covered by its publishing guide | Not covered by its publishing guide | When authentication is required: a 401 (not 403) with an RFC 9728 challenge. An optional server card at `/.well-known/mcp/server-card.json` |

What follows for Maidan:

1. Metadata documents cover ChatGPT, Claude and VS Code. Dynamic registration
   is still the only automatic path for Cursor and Gemini CLI, so it stays.
2. A metadata document parser must accept the list-shaped
   `token_endpoint_auth_methods_supported`, or ChatGPT cannot connect.
3. The 401 with `WWW-Authenticate` is required by Claude and Smithery and is
   the discovery path for Gemini CLI. Maidan's 401 carries no challenge today.
4. Refresh rotation with `invalid_grant` on failure, and token responses inside
   Claude's 10 second budget, are interoperability requirements, not polish.
5. Directory review needs a demo account a reviewer can sign in to with a
   password and no second factor. On a self-hosted product that means the
   hosted demo instance of Next 13 with an account in its identity provider,
   never a credential routed through a model.

## What identity providers support today

The local reference run below exercised Keycloak. The rest comes from each
project's documentation and repository.

| Provider (license) | Resource indicators | Metadata documents | Dynamic registration | Notes |
| --- | --- | --- | --- | --- |
| Keycloak 26.8 (Apache-2.0) ([guide](https://www.keycloak.org/securing-apps/mcp-authz-server)) | Experimental feature `resource-indicators`; the resource must be a resource server client's `resource_url` | Experimental feature `cimd`, through a client policy executor with a domain allow list | Yes, anonymous with trusted-host policies | Refuses ChatGPT's list-shaped document (reproduced below). Claude Desktop's document needs an extra executor option. The guide lists 2026-07-28 support as experimental |
| ORY Hydra (Apache-2.0) ([releases](https://github.com/ory/hydra/releases), [metadata document issue](https://github.com/ory/hydra/issues/4061)) | Merged in its OAuth library in May 2026, after the latest Hydra release (v26.2.0, March 2026) | Open issue | Yes | Headless: the operator builds the login and consent pages |
| Zitadel (AGPL-3.0) ([repo](https://github.com/zitadel/zitadel)) | Not established | Not established | Yes, with an option for unauthenticated registration | The AGPL license is a problem for some operators |
| Authentik ([repo](https://github.com/goauthentik/authentik)) | Not established | Not established | Requires a bearer token with a registration scope, so not anonymous by default | |
| Auth0 (hosted) | Not established | Yes, per OpenAI's auth guide | Yes | A hosted service, not a self-hosting answer |

None of the self-hostable providers gives an operator the full 2026-07-28 set
in a stable release. That fact decides a lot of what follows.

## Options compared

| | A. Embedded authorization server (recommended) | B. Delegate to the operator's OIDC provider (Maidan as resource server only) | C. A separate authorization server beside Maidan (Hydra or similar) |
| --- | --- | --- | --- |
| **Security** | Tokens stay opaque and hashed, revocation is immediate, audience and workspace are checked against Maidan's own rows, and consent can show Maidan's evidence. The cost is a new security-critical surface that needs review | The provider's JWTs live until expiry unless every request is introspected. The provider knows nothing of workspaces or capabilities, so scopes would need a mapping table per provider. [OIDC](OIDC.md) already lists provider JWT passthrough as rejected, because provider capabilities are not Maidan capabilities and workspace scope is harder | A second service with its own database and admin API to secure. Login and consent are still Maidan's to build |
| **Operator burden** | Nothing new to run. Uses the provider they already set up for console sign-in, or none | Every operator must configure their provider for MCP: resource indicators, metadata documents, dynamic registration, audience mappers. Each provider does this differently, and most cannot yet | Another container, its database and migrations, upgrades, and the wiring between the two |
| **Self-hosting fit** | Best. Works on a single binary and on a Raspberry Pi | Depends on the provider. Keycloak works only with experimental features and still fails ChatGPT | Poor for small installs. Doubles the moving parts |
| **License** | Maidan's own | The provider's | Hydra is Apache-2.0; Zitadel is AGPL-3.0 |
| **Rust ecosystem** | No maintained authorization server crate covers resource indicators or metadata documents (`oxide-auth` 0.6.1, last released June 2024, has neither). Built on what Maidan already uses: the `openidconnect` crate for the upstream sign-in, `sha2` hashing and its existing PKCE code | Needs JWT validation (`jsonwebtoken`) and per-provider configuration | Needs an HTTP client to Hydra's admin API and the login and consent pages |
| **Directory reach today** | ChatGPT, Claude, VS Code, Cursor and Gemini CLI, once built | Bounded by the provider. With Keycloak 26.8, not ChatGPT | Bounded by Hydra: no metadata documents, no resource indicators in a release |

Option B stays available in one narrow form after A: an operator who insists
can point the protected resource metadata at their own provider, but Maidan
still refuses tokens it did not issue, so that form only works through the
Enterprise-Managed Authorization extension, where the provider's assertion is
exchanged for a Maidan grant. That is later work.

## Recommended architecture

### Shape

The authorization server lives in `maidan-server` on the same origin as the
MCP endpoints. Its issuer is the public origin of the instance, with no path,
so its metadata sits at `/.well-known/oauth-authorization-server` and every
client's first discovery attempt succeeds.

| Endpoint | Purpose |
| --- | --- |
| `/.well-known/oauth-protected-resource/mcp` (one per MCP endpoint) | RFC 9728 metadata naming the issuer, `scopes_supported` (minimal) and the bearer method |
| `/.well-known/oauth-authorization-server` | RFC 8414 metadata: `S256` only, `none` and later `private_key_jwt`, `client_id_metadata_document_supported`, `authorization_response_iss_parameter_supported`, the registration and revocation endpoints |
| `/oauth/authorize` | Validates the client and request, signs the person in, picks the workspace, shows consent, returns the code with `state` and `iss` |
| `/oauth/token` | Authorization code with PKCE, and refresh with rotation. Form-encoded, as Claude requires |
| `/oauth/register` | Dynamic registration for clients without a metadata document |
| `/oauth/revoke` | RFC 7009 revocation |

Every MCP endpoint (`/mcp`, `/mcp/streamable`, `/mcp/worker`, `/mcp/reviewer`)
answers an unauthenticated request with 401 and
`WWW-Authenticate: Bearer resource_metadata="…"`, plus the scope the request
needs. Each endpoint is its own resource, so a token for `/mcp/worker` is
refused at `/mcp`.

### Who signs in

The person consenting is authenticated the way the console already does it:

- with an OIDC provider configured, by Maidan's relying-party flow
  ([OIDC](OIDC.md)), which already binds the workspace into `state` and keys
  identities by `(workspace, issuer, subject)`;
- without one, by a pasted member token exchanged for a short session, the
  console's existing fallback. The token itself never reaches the client.

Maidan never becomes a password store. The consenting member must be a human
member of the workspace; an agent member cannot consent.

### Workspace selection

A grant binds exactly one workspace. On an instance with one workspace there is
nothing to choose. Otherwise the person names the workspace before signing in,
and the sign-in runs for that workspace, because identities are per workspace
and Maidan has no cross-workspace identity on purpose. A list of every
workspace the signed-in subject belongs to is possible but is a maintainer
decision, since it is a cross-workspace lookup. The issued token is a normal
workspace-bound Maidan token, so every existing tenant check applies unchanged.

### Clients

- **Metadata documents first.** A `client_id` that is an HTTPS URL is fetched
  through Maidan's existing egress guard (`maidan-auth`'s egress module, which
  already refuses private and special-use addresses), with no redirects, a 5 KB
  cap, short timeouts, a bounded cache that never stores failures, and a
  loopback exception only in development builds. The parser accepts both
  `token_endpoint_auth_method` and the list-shaped
  `token_endpoint_auth_methods_supported`. Consent shows the client's name, its
  domain and the redirect host, and warns on loopback redirects.
- **Dynamic registration as the fallback**, for Cursor and Gemini CLI. Public
  clients only (`none`), redirect URIs limited to loopback or HTTPS, a rate limit
  per source, an expiry for registrations that never complete a grant, and
  consent always shown.
- **Pre-registered clients** through an admin call, for operators who want an
  allow list.

### Tokens and grants

A grant row records the client, the member, the workspace, the resource, the
consented scopes, the upstream identity and its sign-in time, and an expiry.
Access tokens are ordinary `api_tokens` rows with a `maid_` secret and a
reference to the grant, so the per-request lookup that resolves every Maidan
token resolves these too, and a revoked or expired grant fails the lookup
immediately. Refresh tokens are hashed, belong to a family, rotate on every use,
and a reused one revokes its family, which answers `invalid_grant`. There are no
JWTs and no signing keys to rotate.

The legacy installed-app exchange (`POST /oauth/app/token` in
`crates/maidan-server/src/app_oauth.rs`) is a narrower version of the same
thing. The build folds it into the general server rather than keeping two code
paths, if the maintainer agrees.

## Scopes, capability presets and workspaces

- **Scopes are capability strings** from the delegatable work set in
  `maidan-auth`: `workspace:read`, `workspace:write`, `message:post`,
  `thread:transition`, `artifact:upload`, `search:query`, `event:subscribe`.
  `maidan.agent.worker` is accepted as shorthand for all seven, which is the
  scope hierarchy the specification asks servers to honor.
- **Authority capabilities are never grantable**: `token:admin`,
  `channel:admin`, the secret and federation capabilities, global audit,
  `operator:global` and `approval:grant`. A request for one is refused with
  `invalid_scope`. An OAuth client cannot mint tokens, read secrets or approve
  as a trusted automated approver.
- **Effective capabilities** are requested ∩ consented ∩ the workspace's OAuth
  ceiling. The ceiling defaults to the worker set and a workspace admin can lower
  it. Nothing can raise a grant above the consenting member's own
  capabilities.
- **`scopes_supported` is minimal**: `workspace:read`. Wider scopes arrive
  through 403 `insufficient_scope` challenges naming exactly what a call needs,
  so a client that only reads never holds write.
- **Profiles stay tool filters.** `/mcp/worker` and `/mcp/reviewer` keep
  choosing which tools are listed; the token's scopes decide what succeeds.

## Revocation and expiry

| Event | Effect |
| --- | --- |
| Access token lifetime | Default 1 hour. Short enough that a stolen token ages out, long enough to stay inside one session |
| Refresh family lifetime | Capped by the delegation ceiling of 90 days. Idle families expire sooner |
| The person disconnects the app, or an admin revokes the grant | The grant row is revoked; every access and refresh token under it fails on its next lookup |
| `POST /oauth/revoke` | Revokes the token's grant family, per RFC 7009 |
| The member leaves the workspace, is suspended, or loses a capability | Leaving or suspension revokes the member's grants. Capabilities are re-intersected at lookup, so a removed capability disappears from the grant at once |
| The workspace lowers its OAuth ceiling | Applies on the next lookup |
| A refresh token is reused | The family is revoked, and the event is audited |

## `approval_decide` under OAuth

The maintainer decided on 2026-10-08 that platform surfaces may record
approvals through a model-callable `approval_decide` tool. Under OAuth the risk
is concrete. The platform's model holds a token that acts for a human member.
Today's gate check accepts any token whose member is human, so without more,
the model that did the work could approve it on a single "looks good" from the
person, or with no word from them at all. These mitigations close that, and are
the proposed defaults for Next 14 and Next 17.

1. **Who the token acts for.** An OAuth token acts for the consenting human
   member, and each grant carries its own actor identity, so Maidan records
   every action as "member, via grant G of client C". Audit and the gate record
   both show it.
2. **Separation between producing and approving.** The existing rule refuses an
   approval from whoever requested the gate, by member or actor. Under OAuth the
   actor is the grant, so a gate requested through grant G cannot be accepted
   through grant G. The build extends the rule to any grant that worked the
   thread under review.
3. **A separate scope.** Deciding needs `approval:decide`, which is never in
   `scopes_supported` and never in the worker shorthand. A client must ask for
   it, and consent names it on its own line.
4. **Per-decision step-up.** Holding `approval:decide` is not enough. A call to
   `approval_decide` without a decision token answers 403 `insufficient_scope`
   with the dynamic scope `approval:decide:<gate id>`. The client re-authorizes;
   Maidan's authorization page requires a fresh upstream sign-in (`max_age`
   short), shows the gate's evidence packet and the proposed decision, and the
   person approves or declines there. The resulting token is good for that gate
   only, once, for at most 10 minutes, with no refresh. The decision is a
   human's act on Maidan's page, not the model's.
5. **Clients that cannot step up** get the same gate as a console link (or URL
   elicitation where the client supports it), never a weaker path.
6. **Evidence binding.** The decision token is bound to the evidence root the
   page showed, so a gate whose evidence changes after the person looked cannot
   be decided with it.
7. **`approval:grant` stays admin-minted.** Automated approvers keep the
   explicit admin path. OAuth cannot reach it.

## Threats and mitigations

| Threat | Mitigation |
| --- | --- |
| A token minted for another server is replayed at Maidan | Maidan accepts only its own opaque tokens, checks the grant's resource against the endpoint, and refuses everything else with 401 |
| Server-side request forgery through a metadata document URL | The egress guard, no redirects, a size cap, timeouts and a bounded cache |
| Mass registration through dynamic registration | Rate limits, expiry for unused registrations, consent always shown, an operator switch to turn it off |
| Authorization code interception | `S256` PKCE required, `plain` refused, codes single-use and short-lived, exact redirect matching |
| Mix-up between authorization servers | `iss` in every authorization response |
| A consent in workspace A yields access in B | One workspace per grant, workspace-bound tokens, and two-workspace tests in every build PR |
| A model approves its own work | The `approval_decide` mitigations above |
| A stolen refresh token | Rotation with reuse detection, family revocation, the 90 day cap |

## Maintainer decisions

Each has a recommended default. None is settled by this page.

| Decision | Options | Recommended default |
| --- | --- | --- |
| Architecture | Embedded, delegate to the operator's provider, or a sidecar | Embedded, with sign-in delegated to the operator's provider |
| Public origin and issuer | A new general public-origin setting, or reuse the A2A public origin | A new general setting (for example `MAIDAN_PUBLIC_ORIGIN`), with the A2A one reading from it later |
| Whether the server is on by default | On whenever the public origin is set, or behind its own switch | Behind its own switch until the security review passes, then on when the origin is set |
| Dynamic registration | On or off by default | On, rate-limited, with an operator switch |
| Metadata document trust policy | Any HTTPS domain, or an allow list | Any HTTPS domain with the fetch guards, plus an optional allow list |
| Client authentication | `none` only, or `private_key_jwt` too | `none` first; `private_key_jwt` when a client needs it |
| Scope vocabulary | Capability strings, or a separate OAuth vocabulary | Capability strings, with the worker shorthand |
| OAuth ceiling | The worker set, read only, or per workspace from the start | The worker set, lowerable per workspace |
| Access token lifetime | 15 minutes to 1 hour | 1 hour |
| Refresh family cap | 30 days, or the 90 day delegation ceiling | 90 days |
| Workspace per grant | One workspace per grant, or several | One, chosen before sign-in; a cross-workspace list stays off |
| Per-workspace resource URLs | One resource per MCP endpoint, or per workspace too | Per endpoint now; per-workspace URLs later if a directory needs them |
| The installed-app exchange | Fold into the general server, or keep | Fold in, with no compatibility shim |
| Step-up for `approval_decide` | Required, or optional per workspace | Required |
| Consent for every new client | Always, or remembered per client | Always for registered and document clients; remembered only for pre-registered ones |
| Security review | Before any listing depends on it | 3 to 5 days, after build PR 7 and before Lanes 10 and 11 |

## Build plan

Each PR is independently reviewable, ships with its tests, and leaves `main`
working. Sizes use the Open Work scale. "Isolation" means two workspaces and
two members exercised in the same test.

| # | PR | Size | Crates | Tests |
| --- | --- | --- | --- | --- |
| 1 | Protected resource metadata and the 401 challenge, behind the public-origin setting | S | `maidan-server`, `maidan-env` | Metadata per MCP endpoint; `resource` equals the endpoint URL; 401 carries `resource_metadata` on every MCP route and nowhere else; no change when the setting is unset; the smoke script's first two steps turn PASS |
| 2 | Store: clients, grants, codes and refresh families, with migrations on both backends | M | `maidan-store`, `maidan-types` | Postgres and SQLite parity; a grant cascades to its tokens; isolation: a grant in workspace A is invisible to every workspace B query |
| 3 | Token resolution: grant liveness, resource audience and capability re-intersection | M | `maidan-auth`, `maidan-store` | Revoked or expired grant fails the next request; audience mismatch is 401; removed member capabilities disappear at once; isolation: an A token at B's resources is refused without an existence oracle |
| 4 | The authorization server core: metadata, authorize with sign-in, workspace choice and consent, the token endpoint, scope mapping and the 403 `insufficient_scope` challenge | M to L | `maidan-server`, `maidan-auth`, `maidan-store` | `S256` only; exact redirects; `state` and `iss`; `invalid_target`; authority scopes refused; codes single-use; isolation: consent in A cannot produce a token usable in B, and the workspace choice offers nothing the person is not a member of |
| 5 | Client registration: metadata documents and dynamic registration | M | `maidan-server`, `maidan-auth` | Egress guard refuses private addresses and redirects; size cap; failures not cached; ChatGPT's list-shaped document accepted; registration rate limit and expiry; isolation: a client registered while consenting in A gains nothing in B |
| 6 | Revocation and the connected-apps view: revoke endpoint, member and admin views of grants, cascades, the per-workspace ceiling | S to M | `maidan-server`, `maidan-store`, the console | RFC 7009; refresh reuse revokes the family; member removal ends grants; isolation: an admin of A cannot list or revoke B's grants |
| 7 | Step-up and `approval_decide` safety | M | `maidan-mcp`, `maidan-server`, `maidan-auth`, `maidan-store` | A decision without a step-up token is 403 with the gate scope; the token decides one gate once; the requesting grant cannot decide; evidence change invalidates it; isolation: a step-up for a gate in B cannot decide A's gate |
| 8 | Docs and client recipes: ChatGPT, Claude, Cursor, VS Code and Gemini CLI against a local instance; the smoke script run with `STRICT=1`; threat model rows | S | docs, `scripts` | Every recipe run and recorded; the smoke script has no GAP lines |

Then the security review, then the directory lanes. Every PR that adds a route
updates the tenant isolation suite's route accounting.

The smoke script names these PR numbers in its GAP lines, so its output reads as
a progress report.

## Local setup

Two scripts and a reference realm, added with this page. They change nothing in
the server.

- [`examples/keycloak/maidan-mcp-reference-realm.json`](../examples/keycloak/maidan-mcp-reference-realm.json)
  is a Keycloak realm configured as close to the 2026-07-28 requirements as
  Keycloak gets: a resource server client whose `resource_url` is the local
  Maidan's `/mcp`, a pre-registered public client with consent, anonymous
  dynamic registration limited to loopback hosts, a metadata document policy for
  loopback documents, rotating refresh tokens and 5 minute access tokens. It has
  no users and no secrets.
- [`examples/oauth-dev/compose.yaml`](../examples/oauth-dev/compose.yaml) runs
  Keycloak 26.8.0, pinned by digest, with the `cimd` and `resource-indicators`
  features on.
- [`scripts/oauth-dev-provider.sh`](../scripts/oauth-dev-provider.sh) starts it
  through Docker Compose or, without Docker, a Keycloak distribution
  (`KC_HOME`), imports every realm in `examples/keycloak` through the admin API
  with `MAIDAN_URL` substituted (the console sign-in realm from the Keycloak
  recipe in [OIDC](OIDC.md) included, when present), and creates a user with a
  generated password.
- [`scripts/mcp-oauth-smoke.sh`](../scripts/mcp-oauth-smoke.sh) walks the flow
  with curl the way an MCP client does and prints PASS, GAP, NOTE or FAIL per
  step. Until Maidan publishes its own metadata it runs the authorization
  server steps against the reference realm; once Maidan does, it follows
  Maidan's metadata instead, so the same script tests the real server.

```sh
MAIDAN_URL=http://127.0.0.1:8080 scripts/oauth-dev-provider.sh up
OAUTH_ISSUER=http://127.0.0.1:8081/realms/maidan-mcp-reference OAUTH_USER=ada \
  OAUTH_PASSWORD_FILE=/tmp/maidan-oauth-dev/user.pw MAIDAN_URL=http://127.0.0.1:8080 \
  scripts/mcp-oauth-smoke.sh
scripts/oauth-dev-provider.sh down
```

The compose file uses host networking, because Keycloak fetches metadata
documents the smoke script serves on loopback, and its issuer must be the same
address the shell and Maidan use. Docker Desktop needs host networking enabled.

### Run record, 2026-10-08

A local Maidan built from `main` at `73546747` ran on `127.0.0.1:8090` with
SQLite and authentication on. Keycloak 26.8.0 ran on `127.0.0.1:8091` through
Docker Compose, and again on `127.0.0.1:8092` from the distribution on Java 21.
Both runs gave the same result: 0 failed, 4 gaps.

| Step | Result |
| --- | --- |
| Unauthenticated `POST /mcp` | 401 with no `WWW-Authenticate`: GAP, build PR 1 |
| Protected resource metadata | None at either well-known URI: GAP, build PR 1 |
| Authorization server metadata | Found at the RFC 8414 path-inserted URI, issuer matches, `S256`, `iss` and metadata documents with `none` advertised: PASS. Keycloak also advertises `plain`, which Maidan's server will not |
| Dynamic registration | Anonymous public client registered: PASS |
| Authorization code with PKCE and `resource` | Sign-in and consent through Keycloak's pages, `state` and `iss` returned, the token's audience is exactly `http://127.0.0.1:8090/mcp`: PASS |
| Wrong verifier | `invalid_grant`: PASS |
| Foreign resource | Keycloak issued the code and refused at the token request with `invalid_target`: PASS. Maidan's server should refuse at the authorization request |
| Refresh rotation and reuse | New refresh token issued, the used one refused with `invalid_grant`: PASS |
| Revocation | The revoked refresh token refused with `invalid_grant`: PASS |
| Metadata document client served on loopback | Full flow, audience correct: PASS |
| ChatGPT-shaped document (list-shaped `token_endpoint_auth_methods_supported`) | Keycloak refused it: its parser rejects the unknown field and reports the fetch as failed. NOTE: Maidan's parser must accept it |
| A Keycloak token presented to Maidan | 401: PASS. Maidan does not accept tokens it did not issue |
| Maidan's own tokens, and `insufficient_scope` | GAP, build PRs 2 to 5 (tokens) and 4 (the challenge) |

What the run taught about the tools themselves:

- Keycloak issues a token naming the resource only when the client's tokens
  already carry the resource server's audience. Metadata document clients get
  that from their executor. Dynamically registered clients need a realm default
  client scope with an audience mapper, and a realm file that declares client
  scopes replaces Keycloak's built-in ones, so the provider script adds that
  scope through the admin API after import.
- Keycloak imports a directory only from files named `<realm>-realm.json`, and
  a mounted directory is invisible when the Docker daemon runs on another
  filesystem. The provider script imports through the admin API instead.

## Sources

Specifications:

- MCP authorization, [2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
  with its [discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery),
  [client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration)
  and [security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations)
  pages; [2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization),
  [2025-06-18](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization),
  [2025-03-26](https://modelcontextprotocol.io/specification/2025-03-26/basic/authorization);
  [Enterprise-Managed Authorization](https://modelcontextprotocol.io/extensions/auth/enterprise-managed-authorization).
- [OAuth 2.1](https://datatracker.ietf.org/doc/draft-ietf-oauth-v2-1/),
  [Client ID Metadata Document](https://datatracker.ietf.org/doc/draft-ietf-oauth-client-id-metadata-document/),
  [RFC 9728](https://www.rfc-editor.org/rfc/rfc9728),
  [RFC 8414](https://www.rfc-editor.org/rfc/rfc8414),
  [RFC 8707](https://www.rfc-editor.org/rfc/rfc8707),
  [RFC 7591](https://www.rfc-editor.org/rfc/rfc7591),
  [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636),
  [RFC 9207](https://www.rfc-editor.org/rfc/rfc9207),
  [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750),
  [RFC 7009](https://www.rfc-editor.org/rfc/rfc7009),
  [RFC 7662](https://www.rfc-editor.org/rfc/rfc7662).

Clients and directories:

- [OpenAI Apps SDK authentication](https://developers.openai.com/apps-sdk/build/auth)
  and [submission guidelines](https://developers.openai.com/apps-sdk/app-submission-guidelines).
- [Claude connector authentication](https://claude.com/docs/connectors/building/authentication).
- [Cursor MCP](https://cursor.com/docs/mcp) and [plugins](https://cursor.com/docs/plugins).
- [VS Code MCP guide](https://code.visualstudio.com/api/extension-guides/ai/mcp).
- [Gemini CLI MCP servers](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md).
- [MCP Registry](https://github.com/modelcontextprotocol/registry).
- [Smithery publishing](https://smithery.ai/docs/build/publish).

Identity providers and libraries:

- [Keycloak as an MCP authorization server](https://www.keycloak.org/securing-apps/mcp-authz-server)
  and [client registration](https://www.keycloak.org/securing-apps/client-registration).
- [ORY Hydra releases](https://github.com/ory/hydra/releases) and
  [its metadata document issue](https://github.com/ory/hydra/issues/4061).
- [Zitadel](https://github.com/zitadel/zitadel), [Authentik](https://github.com/goauthentik/authentik).
- [oxide-auth](https://docs.rs/oxide-auth).

Inside the repo: [OIDC](OIDC.md), [Capability map](Capability%20Map.md),
[Threat model](Threat-Model.md), [Decisions](Decisions.md),
[Open Work](Open%20Work.md) (Next 14 to 17 and Later CA).
