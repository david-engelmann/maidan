# Gemini app

The Gemini app adds a custom app by MCP server URL under Settings, Connected
Apps, Custom apps. For a server without dynamic client registration, it takes a
client id and secret under Advanced features
([Gemini Apps Help](https://support.google.com/gemini/answer/17209137)). Custom
apps need a personal Google account, 18 or over, in the US.

Google's guide says the Gemini app currently requires a `registration_endpoint`
in the authorization server metadata, even when the server supports client
metadata documents, and that the client id and secret fields are the way
around it
([Connected Apps OAuth guide](https://developers.google.com/connected-apps/mcp/oauth)).
Maidan publishes no registration endpoint, so this recipe relies on those
fields. The guide lists the redirect hosts Google uses
(`https://oauth-redirect.googleusercontent.com/r/...` and others) but not the
exact URI, and Maidan matches redirect URIs exactly, so the first attempt is
how you learn it.

Not yet run end to end against Maidan. Record the commit or image digest and
the result when it is.

## Operator

1. Set `MAIDAN_PUBLIC_ORIGIN` and a sign-in method.
2. Pre-register Gemini as a confidential client: the `gemini` entry in
   [`clients.example.json`](clients.example.json), with the SHA-256 of a fresh
   secret. Leave the redirect URI as the placeholder for now.
3. Restart the instance.

## Person

1. On gemini.google.com, open Settings, then Connected Apps, and under Custom
   apps add a custom app.
2. Enter `https://<your instance>/mcp/streamable`. Next to Advanced features,
   click Show more and enter the client id `gemini` and the secret.
3. Click Next. Maidan answers `redirect_uri mismatch`: copy the `redirect_uri`
   query parameter from that page's address, URL-decoded.
4. The operator puts that URI in the client's `redirect_uris`. Because
   existing client ids are skipped at boot, give the entry a new id (for
   example `gemini-2`), restart, and start again from step 1 with that id.
5. Sign in to Maidan and approve the scopes. Gemini asks you to confirm each
   write before it runs.

If the attempt fails with `invalid_target` instead, Gemini did not send the
resource indicator and cannot connect until it does.

## Gemini CLI and Gemini Enterprise

Gemini CLI sends a bearer header; see [docs/Clients.md](../../docs/Clients.md).
Gemini Enterprise's custom MCP data store takes the authorization URL
(`<origin>/oauth/authorize`), token URL (`<origin>/oauth/token`), a client id
and secret and PKCE, entered by an admin; register it the same way.
