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

export function snippets(base: string): Snippet[] {
  const url = mcpUrl(base);
  return [
    { id: "json", title: "MCP config (JSON)", text: JSON.stringify({ mcpServers: { maidan: server(base) } }, null, 2) },
    { id: "claude", title: "Claude Code", text: `claude mcp add --transport http maidan ${url} --header "Authorization: Bearer $MAIDAN_TOKEN"` },
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
