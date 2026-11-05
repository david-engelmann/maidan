// One-click client setup for a fresh sandbox. Pure, so the Worker's tests
// check every format without a browser (test/try.test.js).
//
// Formats, as each client documents them:
// - Claude Code: `claude mcp add --transport http <name> <url> --header "…"`.
// - Cursor: cursor://anysphere.cursor-deeplink/mcp/install?name=<name>&config=<base64 JSON>,
//   where the JSON is the server's mcp.json entry ({url, headers}).
// - VS Code: vscode:mcp/install?<URI-encoded JSON>, the JSON `code --add-mcp`
//   takes ({name, type: "http", url, headers}).
// - Codex: an [mcp_servers.<name>] table in ~/.codex/config.toml whose
//   bearer_token_env_var names the variable holding the token.
//
// The Cursor and VS Code links carry the token, because a one-click install
// has nowhere else to put it. They are custom-scheme links the browser hands
// to the local app; no request carries them to a server.

export const SERVER_NAME = "maidan";
export const TOKEN_VAR = "MAIDAN_TOKEN";

function base64(text) {
  if (typeof btoa === "function") return btoa(text);
  return Buffer.from(text, "utf8").toString("base64");
}

export function claudeCommand(mcpUrl, token) {
  return `claude mcp add --transport http ${SERVER_NAME} ${mcpUrl} --header "Authorization: Bearer ${token}"`;
}

export function cursorLink(mcpUrl, token) {
  const config = { url: mcpUrl, headers: { Authorization: `Bearer ${token}` } };
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=${SERVER_NAME}&config=${encodeURIComponent(base64(JSON.stringify(config)))}`;
}

export function vscodeLink(mcpUrl, token) {
  const config = { name: SERVER_NAME, type: "http", url: mcpUrl, headers: { Authorization: `Bearer ${token}` } };
  return `vscode:mcp/install?${encodeURIComponent(JSON.stringify(config))}`;
}

export function codexConfig(mcpUrl, token) {
  return [
    "# In your shell, before you start Codex:",
    `export ${TOKEN_VAR}=${token}`,
    "",
    "# In ~/.codex/config.toml:",
    `[mcp_servers.${SERVER_NAME}]`,
    `url = "${mcpUrl}"`,
    `bearer_token_env_var = "${TOKEN_VAR}"`,
  ].join("\n");
}
