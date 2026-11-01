# Connect a chat assistant to Maidan

Recipes for the assistants that add a remote MCP server by URL and sign in with
OAuth: claude.ai, Perplexity, grok.com and the Gemini app. Coding agents that
send a bearer header (Claude Code, Cursor, Gemini CLI, Copilot CLI) are in
[docs/Clients.md](../../docs/Clients.md).

| Assistant | How it identifies itself to Maidan | What the operator sets up | Status |
|---|---|---|---|
| [claude.ai custom connector](claude.md) | Client metadata document hosted on `claude.ai` | `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS=claude.ai` | The flow from #1789; the hosted instances list `claude.ai` |
| [Perplexity](perplexity.md) | A confidential client you pre-register | One entry in `MAIDAN_OAUTH_CLIENTS` | Not yet run end to end |
| [grok.com](grok.md) | A public client you pre-register | One entry in `MAIDAN_OAUTH_CLIENTS` | Not yet run end to end |
| [Gemini app](gemini.md) | A confidential client you pre-register | One entry in `MAIDAN_OAUTH_CLIENTS` | Not yet run end to end; the redirect URI must be read from the first attempt |

## What every recipe shares

- **The URL is `https://<your instance>/mcp/streamable`.** OAuth access tokens
  are bound to that resource (RFC 8707) and are refused on `/mcp` and on the
  REST API.
- **The instance needs `MAIDAN_PUBLIC_ORIGIN`** set to its public HTTPS origin,
  and a sign-in method, so the authorization endpoint can send a signed-out
  browser to sign in. Without the origin, every OAuth endpoint answers 404.
- **Maidan has no dynamic client registration, by choice**
  ([docs/OAuth.md](../../docs/OAuth.md)). An assistant that can only register
  itself dynamically gets no client id. Every assistant here either fetches a
  client metadata document (claude.ai) or takes a client id and secret typed
  into its connector dialog.
- **A grant never exceeds the person who consents.** The scopes are Maidan
  capabilities, and a client's `allowed_scopes` caps them further. Start with
  `workspace:read`; add `workspace:write` when the assistant should post.
- **Approvals stay in the console.** An OAuth token never decides an approval
  gate; the person opens the link the assistant shows and decides it there.
- **The authorize request must name the resource.** Maidan refuses an
  authorize request without `resource=<origin>/mcp/streamable`
  (`invalid_target`). The MCP authorization spec requires clients to send it.
  If an assistant's first attempt fails with `invalid_target`, it does not, and
  it cannot connect until it does.

## Pre-registering a client

`MAIDAN_OAUTH_CLIENTS` is a JSON array read at boot. Existing client ids are
skipped, so it is safe to leave set. [`clients.example.json`](clients.example.json)
has one entry per assistant. For a confidential client, generate a secret, give
the assistant the secret, and give Maidan only its SHA-256 in hex:

```sh
secret="$(openssl rand -hex 32)"
printf %s "$secret" | sha256sum | cut -d' ' -f1   # client_secret_hash
```

Keep the secret in a password manager, not in the repository or the instance's
environment. Redirect URIs must match exactly, so copy them character for
character, trailing slash included.
