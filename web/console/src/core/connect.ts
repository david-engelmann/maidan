// Connect an agent: the MCP endpoint and copy-ready snippets. OAuth comes
// first because the clients people use sign in through the browser and need
// no token; a token is the fallback for scripts, your own loop, and clients
// without OAuth. A snippet never carries a real token; it names a placeholder
// the person fills in.
export const TOKEN_PLACEHOLDER = "REPLACE_WITH_MAIDAN_TOKEN";

/** The clients that sign in through the browser when given the URL (docs/Clients.md). */
export const OAUTH_CLIENTS = "Cursor, VS Code, Codex, Zed or Claude Code";

/** Who still needs a token: everything that cannot open a browser to sign in. */
export const TOKEN_FALLBACK = "A token is for scripts, your own loop, and clients without OAuth.";

/** `token` marks a snippet that needs a token pasted in; the rest work as they are. */
export type Snippet = { id: string; title: string; text: string; token: boolean };

export function mcpUrl(base: string): string {
  return `${base.replace(/\/$/, "")}/mcp/streamable`;
}

function server(base: string) {
  return { url: mcpUrl(base), headers: { Authorization: `Bearer ${TOKEN_PLACEHOLDER}` } };
}

/** The OAuth-first instruction, as one line of plain text. */
export function oauthLine(base: string): string {
  return `Paste ${mcpUrl(base)} into ${OAUTH_CLIENTS} and sign in when the browser opens.`;
}

/** Claude Code with OAuth: no token; a browser opens to sign in on first use. */
export function claudeOAuthCommand(base: string): string {
  return `claude mcp add --transport http maidan ${mcpUrl(base)}`;
}

export function snippets(base: string): Snippet[] {
  const url = mcpUrl(base);
  return [
    { id: "url", title: "MCP URL (sign in, no token)", text: url, token: false },
    { id: "claude-oauth", title: "Claude Code (sign in, no token)", text: claudeOAuthCommand(base), token: false },
    {
      id: "prompt",
      title: "Agent prompt",
      text: "You have the maidan MCP server. Loop: claim the next ready task, do the work, record a result, then hand it to review. Post a note on the thread when you are blocked.",
      token: false,
    },
    { id: "json", title: "MCP config (JSON, with a token)", text: JSON.stringify({ mcpServers: { maidan: server(base) } }, null, 2), token: true },
    {
      id: "claude",
      title: "Claude Code (with a token)",
      // The shell expands $MAIDAN_TOKEN when the command runs, so it is set first.
      text: `export MAIDAN_TOKEN=${TOKEN_PLACEHOLDER}\nclaude mcp add --transport http maidan ${url} --header "Authorization: Bearer $MAIDAN_TOKEN"`,
      token: true,
    },
  ];
}

export function cursorDeeplink(base: string, encode: (s: string) => string = btoa): string {
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=maidan&config=${encodeURIComponent(encode(JSON.stringify(server(base))))}`;
}

export function llmsTxt(base: string): string {
  return `${base.replace(/\/$/, "")}/llms.txt`;
}
