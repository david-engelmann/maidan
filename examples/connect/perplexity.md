# Perplexity

Perplexity's custom remote connectors take OAuth, an API key or no
authentication, over Streamable HTTP or SSE. For a server without dynamic
client registration, Perplexity takes a client id and secret under Advanced,
and calls back to a fixed redirect URI
(Perplexity's help center, "Adding custom remote connectors").

Not yet run end to end against Maidan. Record the commit or image digest and
the result when it is.

## Operator

1. Set `MAIDAN_PUBLIC_ORIGIN` and a sign-in method.
2. Pre-register Perplexity as a confidential client: the `perplexity` entry in
   [`clients.example.json`](clients.example.json), with the SHA-256 of a fresh
   secret (see [the README](README.md#pre-registering-a-client)). The redirect
   URI is `https://www.perplexity.ai/rest/connections/oauth_callback`, or
   `https://enterprise.perplexity.ai/rest/connections/oauth_callback` for
   Perplexity Enterprise.
3. Restart the instance so it reads `MAIDAN_OAUTH_CLIENTS`.

## Person

1. In Perplexity, open Add custom, then Add MCP connector, and choose Remote.
   An organization admin may have to allow members to add custom connectors.
2. Name it Maidan, and give it the URL `https://<your instance>/mcp/streamable`.
3. Under Advanced, set Authentication to OAuth, Transport to Streamable HTTP,
   and enter the client id `perplexity` and the secret.
4. Add it, then select the connector card to sign in to Maidan and approve the
   scopes.

## API key instead

Perplexity's API-key option sends a static key. Maidan accepts a member token
only as `Authorization: Bearer <token>`, and Perplexity's docs do not name the
header it uses, so this path is unverified. If you try it, mint a scoped,
expiring member token, never an admin one.
