# grok.com

grok.com adds a custom MCP connector by URL at grok.com/connectors (New
Connector, then Custom), and the server must be reachable over the public
internet (xAI's connector docs). xAI's
docs do not say how Grok identifies itself to a server without dynamic client
registration. Third-party guides report that Grok then shows a form for a
client id, a secret and a token auth method, and that it calls back to
`https://grok.com/connectors-oauth-exchange-code/`. Treat both as unverified
until the recipe is run.

Not yet run end to end against Maidan. Record the commit or image digest and
the result when it is.

## Operator

1. Set `MAIDAN_PUBLIC_ORIGIN` and a sign-in method.
2. Pre-register Grok as a public client (no secret; PKCE only): the `grok`
   entry in [`clients.example.json`](clients.example.json).
3. Restart the instance so it reads `MAIDAN_OAUTH_CLIENTS`.

## Person

1. Go to grok.com/connectors, click New Connector, then Custom. On Grok
   Business or Enterprise, a team admin provisions the connector first.
2. Enter `https://<your instance>/mcp/streamable`.
3. If Grok asks for a client id, enter `grok`, leave the secret empty, and pick
   the PKCE-only (`none`) auth method.
4. Sign in to Maidan in the window that opens and approve the scopes.

If the authorize step fails with `redirect_uri mismatch`, copy the
`redirect_uri` from the address bar of that window into the client's
`redirect_uris`, restart, and try again. If it fails with `invalid_target`,
Grok did not send the resource indicator and cannot connect yet.

## Grok Build and the xAI API

These send a header, so no OAuth client is needed. Mint a scoped, expiring
member token and pass it as `Authorization: Bearer <token>`, for example
`grok mcp add --transport http maidan https://<your instance>/mcp/streamable --header "Authorization: Bearer ${MAIDAN_TOKEN}"` (unverified flag syntax; check `grok mcp add --help`).
