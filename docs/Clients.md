# Connecting MCP clients

How each MCP client connects to Maidan, and what a person checks by hand
before a release. `contracts/mcp-clients.json` in the repository holds the
tested clients, and `client_matrix_e2e` connects to a real server the way each
one does and checks that every client has a recipe here.

Every client below speaks Streamable HTTP. Paste `/mcp/streamable` on your
instance, for example `https://maidan.example.com/mcp/streamable`, into the
client and sign in when the browser opens. No token is copied: the person
consents in the browser, and the client gets a token cut to what that person
holds ([OAuth](OAuth.md)).

## How each client signs in

A client signs in with OAuth in one of two ways, and the instance has to allow
each:

- **By its client metadata document.** The client names itself by an HTTPS
  URL on its maker's host, and the instance fetches its document from there.
  Claude Code and claude.ai (`claude.ai`), Codex and ChatGPT (`chatgpt.com`),
  Zed (`zed.dev`) and VS Code (`vscode.dev`) do this. The host must be in
  `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`. A client that sees the instance
  supports documents never falls back to registering, so a host left out
  breaks that client.
- **By registering itself** (RFC 7591). Cursor, Gemini CLI, the MCP Inspector
  and the SDK examples do this. The instance needs
  `MAIDAN_OAUTH_DYNAMIC_REGISTRATION=1`. The consent page marks such a client
  "unverified app, registered by itself" and names where the browser goes
  next.

maidan.world sets both. A self-hosted instance also needs
`MAIDAN_PUBLIC_ORIGIN`; without it the OAuth endpoints answer 404.

**A token is the fallback**, for scripts, your own loop, and clients without
OAuth (GitHub Copilot CLI below). Add the agent in the console's People
screen, which mints its own token with the `maidan.agent.worker` preset and
fills in the client's setup, or mint one with `maidan init` or the token API
(see [Integration](Integration.md)). The token recipes below read it from the
variable `MAIDAN_TOKEN`; where a client reads environment variables in its
configuration file, the recipe uses that, so the token is never written into
the file. Never commit a project-level MCP configuration that holds a token.

## The client matrix

The tested clients. The JSON file's `auth` is how `client_matrix_e2e`
connects; the OAuth flow itself is proven by the `oauth_*_e2e` tests.

| Client | How it authenticates | Elicitation | Checked by hand each release |
|---|---|---|---|
| Claude Code | OAuth, by its document on claude.ai, or a Bearer header | Supported | No |
| Cursor | OAuth, by registering itself, or a Bearer header | Supported | Yes, because its token handling is unreliable |
| Gemini CLI | OAuth, by registering itself, or a Bearer header | None | No |
| GitHub Copilot CLI | Bearer header | Supported (form since 0.0.421, URL since 0.0.423) | No |
| claude.ai custom connector | OAuth, by its client metadata document | Unverified | Yes |
| ChatGPT developer mode | OAuth, by its document on chatgpt.com, or none against a dev instance | None | Yes |

Each row's facts come from the client's own documentation as read on
2026-10-06 (Gemini CLI's on 2026-10-08), linked in the JSON file. Re-read them
each quarter, and when a client ships a release that changes how it does MCP.
VS Code, Codex and Zed sign in by their documents too and have recipes below;
they are not yet rows in the JSON file.

## Recipes

The examples use `https://maidan.example.com`.

### Claude Code

```sh
claude mcp add --transport http maidan https://maidan.example.com/mcp/streamable
```

Then run `/mcp` in Claude Code and authenticate `maidan`: the browser opens to
sign in with OAuth, and Claude Code keeps the token.

With a token instead:

```sh
claude mcp add --transport http maidan https://maidan.example.com/mcp/streamable \
  --header "Authorization: Bearer $MAIDAN_TOKEN"
```

With a fixed header, a 401 is reported as a failed connection, not as a prompt
to sign in. A rejected or expired token shows up that way.

### Cursor

In `~/.cursor/mcp.json`, or `.cursor/mcp.json` in a project, give only the URL.
Cursor registers itself and opens the browser to sign in with OAuth.

```json
{
  "mcpServers": {
    "maidan": { "url": "https://maidan.example.com/mcp/streamable" }
  }
}
```

With a token instead, add the header. Cursor reads `${env:NAME}` from the
environment, so the token stays out of the file. The console's "Add to
Cursor" link installs this form with the agent's token in it.

```json
{
  "mcpServers": {
    "maidan": {
      "url": "https://maidan.example.com/mcp/streamable",
      "headers": { "Authorization": "Bearer ${env:MAIDAN_TOKEN}" }
    }
  }
}
```

### VS Code

In `.vscode/mcp.json`, or through "MCP: Add Server". VS Code names itself by
its document on vscode.dev and asks you to sign in with OAuth.

```json
{
  "servers": {
    "maidan": { "type": "http", "url": "https://maidan.example.com/mcp/streamable" }
  }
}
```

The console's "Add to VS Code" link installs the token form instead.

### Codex

In `~/.codex/config.toml`, then `codex mcp login maidan` to sign in with OAuth.
Codex names itself by its document on chatgpt.com. A claim can wait up to 300
seconds for work, so the tool timeout is longer than that.

```toml
[mcp_servers.maidan]
url = "https://maidan.example.com/mcp/streamable"
tool_timeout_sec = 330
```

With a token instead, add `bearer_token_env_var = "MAIDAN_TOKEN"` and skip the
login.

### Zed

In Zed's `settings.json`. Zed names itself by its document on zed.dev and
opens the browser to sign in with OAuth.

```json
{
  "context_servers": {
    "maidan": { "url": "https://maidan.example.com/mcp/streamable" }
  }
}
```

### Gemini CLI

```sh
gemini mcp add --scope user --transport http maidan https://maidan.example.com/mcp/streamable
```

Then run `/mcp auth maidan` in Gemini CLI: it registers itself and opens the
browser to sign in with OAuth.

With a token instead:

```sh
gemini mcp add --scope user --transport http -H "Authorization: Bearer $MAIDAN_TOKEN" \
  maidan https://maidan.example.com/mcp/streamable
```

`--scope user` saves the server in your own `~/.gemini/settings.json`. Without
it the command defaults to the project's `.gemini/settings.json`, and the shell
has already expanded the token into the header, so it would sit in plain text
in the project.

Or in `settings.json`, use `httpUrl`, which is Streamable HTTP in every
release. Gemini CLI's documentation still describes a plain `url` as the older
SSE transport, which Maidan does not serve on this path. Release 0.63.0 tries
Streamable HTTP first for a `url` with no `type` and falls back to SSE, so
`httpUrl` is the setting that means the same thing everywhere. Gemini CLI
expands `${MAIDAN_TOKEN}` in the file from the environment; leave `headers`
out to sign in with OAuth instead.

```json
{
  "mcpServers": {
    "maidan": {
      "httpUrl": "https://maidan.example.com/mcp/streamable",
      "headers": { "Authorization": "Bearer ${MAIDAN_TOKEN}" }
    }
  }
}
```

#### As an extension

The repository is a Gemini CLI extension.
[`gemini-extension.json`](../gemini-extension.json) at its root names the
`maidan` MCP server at `${MAIDAN_URL}/mcp/streamable` with the header
`Authorization: Bearer ${MAIDAN_TOKEN}`, and nothing else. It has no host of
its own, so the extension connects to whichever instance `MAIDAN_URL` names,
and the token is read from the environment each time Gemini CLI starts, never
stored by the install.

```sh
export MAIDAN_URL=https://maidan.example.com   # your instance, no trailing slash
export MAIDAN_TOKEN=...                        # a token minted for this agent
gemini extensions install https://github.com/david-engelmann/maidan
gemini extensions list
gemini mcp list
```

`gemini extensions install` clones the repository into
`~/.gemini/extensions/maidan` and asks you to confirm before it installs. Add
`--ref <branch, tag or commit>` to pin a version. Installing from the
repository URL is the supported path, not the extension gallery: the gallery
lists only repositories that carry its topic, and its indexer has skipped valid
repositories before. `gemini extensions update maidan` pulls a newer commit.
`gemini extensions uninstall maidan` removes it.

The extension also loads [`gemini-extension.md`](../gemini-extension.md) as
context for the model: the work loop, and that approvals are decided by a
person signed in to Maidan's console, never in the chat.

Give `MAIDAN_URL` an `https://` address for any instance that is not on your
own machine. The header carries the token on every request, and the manifest
cannot refuse a plain `http://` value, so a token sent to a remote instance
over `http://` can be read and replayed by anyone on the path. Plain
`http://127.0.0.1` is fine for an instance on the same machine.

If either variable is unset when Gemini CLI starts, the server's URL or header
keeps the literal `${...}` text and the connection fails, so `gemini mcp list`
shows `maidan` as disconnected. A server with the same name in your own
`settings.json` takes precedence over the extension's, so remove the
`gemini mcp add` entry above if you switch to the extension.

### GitHub Copilot CLI

Run `/mcp add`, choose HTTP, and give it the URL
`https://maidan.example.com/mcp/streamable` and the header
`Authorization: Bearer <token>`. Copilot CLI saves the server in
`~/.copilot/mcp-config.json`, and its documentation names no way to read the
token from the environment there, so the token sits in that file in plain text.
Keep it private.

### claude.ai custom connector

Add a custom connector in claude.ai with the instance's
`/mcp/streamable` URL and sign in with OAuth: claude.ai identifies itself
with a client metadata document, so the instance needs
`MAIDAN_OAUTH_CLIENT_METADATA_HOSTS=claude.ai` and nothing pre-registered.
Anthropic's servers make the requests, so a firewall in front of the instance
must allow `160.79.104.0/21`. Step by step, with Perplexity, grok.com and the
Gemini app: [examples/connect](../examples/connect/README.md).

### ChatGPT developer mode

Turn on developer mode in ChatGPT's settings, then add the instance's
`/mcp/streamable` URL as an app with OAuth. ChatGPT names itself by its client
metadata document on chatgpt.com, so the instance needs `chatgpt.com` in
`MAIDAN_OAUTH_CLIENT_METADATA_HOSTS`. Against a dev instance it can instead
connect with no authentication and read anonymously (below); its tool list
then shows each read-only tool marked as needing no sign-in.

## A dev instance for clients that cannot send a token

1. Run the instance from `main`, never from a release a team uses, with
   synthetic fixtures only. Its workspace name must begin `synthetic-`.
2. Set `MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE` to that workspace's id. The server
   refuses the variable under `MAIDAN_ENV=production`.
3. Give it a public HTTPS address, and allow `160.79.104.0/21` if a firewall
   fronts it.
4. Name its host in `MAIDAN_ALLOWED_HOSTS`, as a bare host name such as
   `dev.example.com`, with no scheme and no port. A request with no credential
   naming a dotted host not in `MAIDAN_ALLOWED_HOSTS` is refused, so a page on
   another site cannot rebind its name to the instance (see "DNS rebinding" in
   [Production](Production.md)).
5. Record the image digest, or the commit it was built from, at the top of
   every test log. `scripts/mcp-inspector.sh` and `scripts/mcp-conformance.sh`
   print the commit they built.

An anonymous caller reads that one workspace as no member at all. It sees
what is public there, calls only read-only tools, and gets no session. The
details are in the [Integration reference](Integration%20Reference.md#mcp-streamable)
under "Anonymous reading on a dev instance".

## The release check

About five minutes, by a person, before each release.

1. In Cursor, sign in with OAuth, list the tools, and call `whoami`. Restart
   Cursor and call `whoami` again. Do the same with a token. Its token
   handling is unreliable, so a connection that worked once is not taken to
   work after a restart.
2. In ChatGPT developer mode and in a claude.ai custom connector, connect to
   the instance with OAuth, list the tools, and call `whoami`. In ChatGPT
   developer mode, also add the dev instance with no authentication and read
   a channel.
3. Check that a person completes a contested approval correctly from ChatGPT,
   Gemini CLI and claude.ai, which have no elicitation. The agent opens an
   approval gate, the client shows the link to Maidan's console, and the person
   decides it there after reading the evidence.

Record the commit or image digest, the date and the result for each step.

## Approvals stay in the console

Every approval is decided in Maidan's console, on every client. A client may
show that an approval is waiting and link to the console. An answer a client
collects through elicitation is advisory, and no tool a model can call records
an approval.

Who approved is established by the console's own sign-in, through the
instance's identity provider or by a person exchanging their own token for a
browser session there. A reviewer's credential never passes through a model or
a client's chat: no recipe on this page asks for one, and the Gemini CLI
extension's context tells the model never to ask for one.

## What counts as an attributed connect

An attributed connect is an authenticated tool call beyond discovery, made
with a listing's token, and counted once per user per day. A call to
`tools/list` or `initialize` alone is not one, and an anonymous dev read is not
one.
