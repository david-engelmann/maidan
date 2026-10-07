# Connecting MCP clients

How each MCP client this project is tested against connects to Maidan, and
what a person checks by hand before a release. The matrix below is also test
config. `contracts/mcp-clients.json` in the repository holds it,
and `client_matrix_e2e` connects to a real server the way each client does and
checks that every client has a recipe here.

Every client below speaks Streamable HTTP. Point it at `/mcp/streamable` on your
instance (`/mcp` answers the same requests). Most clients send a fixed
`Authorization: Bearer` header with a token minted for the agent. Mint one with
`maidan init` or the token API, and give an agent the `maidan.agent.worker`
preset unless it needs more (see [Integration](Integration.md)). A client that
can send no token reads a dev instance anonymously instead (see
[A dev instance for clients that cannot send a token](#a-dev-instance-for-clients-that-cannot-send-a-token)).

## The client matrix

| Client | How it authenticates | Elicitation | Checked by hand each release |
|---|---|---|---|
| Claude Code | Bearer header | Supported | No |
| Cursor | Bearer header | Supported | Yes, because its token handling is unreliable |
| Gemini CLI | Bearer header | None | No |
| GitHub Copilot CLI | Bearer header | Supported (form since 0.0.421, URL since 0.0.423) | No |
| claude.ai custom connector | None, against a dev instance | Unverified | Yes |
| ChatGPT developer mode | None, against a dev instance | None | Yes |

Each row's facts come from the client's own documentation as read on
2026-10-06, linked in the JSON file. Re-read them each quarter, and when a
client ships a release that changes how it does MCP.

## Recipes

The examples use `https://maidan.example.com` and a token in the environment
variable `MAIDAN_TOKEN`. Where a client reads environment variables in its
configuration file, the recipe uses that, so the token is never written into
the file. A client that stores the header itself keeps it in your home
directory. Keep that file private, and never commit a project-level MCP
configuration that holds a token.

### Claude Code

```sh
claude mcp add --transport http maidan https://maidan.example.com/mcp/streamable \
  --header "Authorization: Bearer $MAIDAN_TOKEN"
```

With a fixed header, a 401 is reported as a failed connection, not as a prompt
to sign in. A rejected or expired token shows up that way.

### Cursor

In `~/.cursor/mcp.json`, or `.cursor/mcp.json` in a project.

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

Cursor reads `${env:NAME}` from the environment, so the token stays out of the
file.

### Gemini CLI

```sh
gemini mcp add --transport http -H "Authorization: Bearer $MAIDAN_TOKEN" \
  maidan https://maidan.example.com/mcp/streamable
```

Or in `settings.json`, use `httpUrl`. A plain `url` selects the older SSE
transport, which Maidan does not serve on this path. Gemini CLI expands
`${MAIDAN_TOKEN}` in the file from the environment.

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

### GitHub Copilot CLI

Run `/mcp add`, choose HTTP, and give it the URL
`https://maidan.example.com/mcp/streamable` and the header
`Authorization: Bearer <token>`. Copilot CLI saves the server in
`~/.copilot/mcp-config.json`, and its documentation names no way to read the
token from the environment there, so the token sits in that file in plain text.
Keep it private.

### claude.ai custom connector

Add a custom connector in claude.ai's connector settings with the dev
instance's URL, and choose No sign in. Anthropic's servers make the requests,
so the instance needs a public HTTPS address, and a firewall in front of it
must allow Anthropic's published egress range `160.79.104.0/21`. The
connector's fixed-credentials option, which would send a token, is not yet
verified against Maidan.

### ChatGPT developer mode

Turn on developer mode in ChatGPT's settings, then add the dev instance's URL
as an app with no authentication. ChatGPT offers OAuth or no authentication,
and Maidan has no OAuth server yet, so this client reads a dev instance
anonymously. Its tool list shows each read-only tool marked as needing no
sign-in.

## A dev instance for clients that cannot send a token

1. Run the instance from `main`, never from a release a team uses, with
   synthetic fixtures only. Its workspace name must begin `synthetic-`.
2. Set `MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE` to that workspace's id. The server
   refuses the variable under `MAIDAN_ENV=production`.
3. Give it a public HTTPS address, and allow `160.79.104.0/21` if a firewall
   fronts it.
4. Record the image digest, or the commit it was built from, at the top of
   every test log. `scripts/mcp-inspector.sh` and `scripts/mcp-conformance.sh`
   print the commit they built.

An anonymous caller reads that one workspace as no member at all. It sees
what is public there, calls only read-only tools, and gets no session. The
details are in [Integration](Integration.md) under "Anonymous reading on a dev
instance".

## The release check

About five minutes, by a person, before each release.

1. In Cursor, connect with a token, list the tools, and call `whoami`. Restart
   Cursor and call `whoami` again. Its token handling is unreliable, so a
   connection that worked once is not taken to work after a restart.
2. In ChatGPT developer mode and in a claude.ai custom connector, add the dev
   instance, list the tools, and read a channel.
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

## What counts as an attributed connect

An attributed connect is an authenticated tool call beyond discovery, made
with a listing's token, and counted once per user per day. A call to
`tools/list` or `initialize` alone is not one, and an anonymous dev read is not
one.
