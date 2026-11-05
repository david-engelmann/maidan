// Connect an agent: the MCP endpoint and copy-ready snippets. A snippet never
// carries a real token; it names a placeholder the person fills in.
export const TOKEN_PLACEHOLDER = "REPLACE_WITH_MAIDAN_TOKEN";

export type Snippet = { id: string; title: string; text: string };

export function mcpUrl(base: string): string {
  return `${base.replace(/\/$/, "")}/mcp/streamable`;
}

function server(base: string) {
  return { url: mcpUrl(base), headers: { Authorization: `Bearer ${TOKEN_PLACEHOLDER}` } };
}

/** Claude Code with OAuth: no token; a browser opens to sign in on first use. */
export function claudeOAuthCommand(base: string): string {
  return `claude mcp add --transport http maidan ${mcpUrl(base)}`;
}

export function snippets(base: string): Snippet[] {
  const url = mcpUrl(base);
  return [
    { id: "json", title: "MCP config (JSON)", text: JSON.stringify({ mcpServers: { maidan: server(base) } }, null, 2) },
    { id: "claude-oauth", title: "Claude Code (sign in, no token)", text: claudeOAuthCommand(base) },
    {
      id: "claude",
      title: "Claude Code (with a token)",
      // The shell expands $MAIDAN_TOKEN when the command runs, so it is set first.
      text: `export MAIDAN_TOKEN=${TOKEN_PLACEHOLDER}\nclaude mcp add --transport http maidan ${url} --header "Authorization: Bearer $MAIDAN_TOKEN"`,
    },
    {
      id: "prompt",
      title: "Agent prompt",
      text: "You have the maidan MCP server. Loop: claim the next ready task, do the work, record a result, then hand it to review. Post a note on the thread when you are blocked.",
    },
  ];
}

export function cursorDeeplink(base: string, encode: (s: string) => string = btoa): string {
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=maidan&config=${encodeURIComponent(encode(JSON.stringify(server(base))))}`;
}

export function llmsTxt(base: string): string {
  return `${base.replace(/\/$/, "")}/llms.txt`;
}
