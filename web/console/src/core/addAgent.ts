// "Add an agent": a name becomes the agent's handle, and once its token is
// minted each client gets the exact setup with that token in it. Kept apart
// from the screens so the shapes are tested. Nothing here stores or logs a
// token; the caller holds it only while the secret panel is open.
import { mcpUrl } from "./connect.ts";
import { trimBase } from "./api.ts";

/** The longest handle the server takes from the console. */
export const HANDLE_MAX = 40;

/** The longest display name the server takes. */
export const NAME_MAX = 100;

/** The handle a typed name becomes: lowercase letters, digits, '-', '_' and '.'. */
export function handleFrom(name: string): string {
  return name
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .trim()
    .replace(/\s+/g, "-")
    .replace(/[^a-z0-9._-]/g, "")
    .replace(/-{2,}/g, "-")
    .replace(/^[-.]+|[-.]+$/g, "")
    .slice(0, HANDLE_MAX)
    .replace(/[-.]+$/, "");
}

/** Why the name cannot be sent, or null when it can. */
export function nameRefusal(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return "Give the agent a name.";
  if (trimmed.length > NAME_MAX) return `Keep the name to ${NAME_MAX} characters.`;
  if (!handleFrom(trimmed)) return "Use at least one letter or digit in the name.";
  return null;
}

export type ClientId = "claude" | "cursor" | "vscode" | "codex" | "loop";

export const CLIENTS: ReadonlyArray<{ id: ClientId; name: string }> = [
  { id: "claude", name: "Claude Code" },
  { id: "cursor", name: "Cursor" },
  { id: "vscode", name: "VS Code" },
  { id: "codex", name: "Codex" },
  { id: "loop", name: "Your own loop" },
];

/** One block of a client's setup: a link to open, or text to copy and run. */
export type SetupStep = { title: string; text: string; href?: string };

/** The one line said beside every setup. */
export const SHOWN_ONCE = "Shown once: the server keeps only a hash, so this token is here only while this panel is open.";

function headers(token: string) {
  return { Authorization: `Bearer ${token}` };
}

/** Cursor's install link: the server config, as JSON, in base64. */
export function cursorInstallLink(base: string, token: string, encode: (s: string) => string = btoa): string {
  const config = { url: mcpUrl(base), headers: headers(token) };
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=maidan&config=${encodeURIComponent(encode(JSON.stringify(config)))}`;
}

/** VS Code's install link: the named server, as URL-encoded JSON. */
export function vscodeInstallLink(base: string, token: string): string {
  const server = { name: "maidan", type: "http", url: mcpUrl(base), headers: headers(token) };
  return `vscode:mcp/install?${encodeURIComponent(JSON.stringify(server))}`;
}

/** The setup for `client`, with this origin and this agent's token filled in. */
export function clientSetup(client: ClientId, base: string, token: string, channelId: string | null = null): SetupStep[] {
  const url = mcpUrl(base);
  switch (client) {
    case "claude":
      return [{ title: "Terminal", text: `claude mcp add --transport http maidan ${url} --header "Authorization: Bearer ${token}"` }];
    case "cursor":
      return [
        { title: "Add to Cursor", text: cursorInstallLink(base, token), href: cursorInstallLink(base, token) },
        { title: "Or in ~/.cursor/mcp.json", text: JSON.stringify({ mcpServers: { maidan: { url, headers: headers(token) } } }, null, 2) },
      ];
    case "vscode":
      return [
        { title: "Add to VS Code", text: vscodeInstallLink(base, token), href: vscodeInstallLink(base, token) },
        { title: "Or in .vscode/mcp.json", text: JSON.stringify({ servers: { maidan: { type: "http", url, headers: headers(token) } } }, null, 2) },
      ];
    case "codex":
      return [
        { title: "Shell, before you start Codex", text: `export MAIDAN_TOKEN=${token}` },
        {
          title: "~/.codex/config.toml",
          text: `[mcp_servers.maidan]\nurl = "${url}"\nbearer_token_env_var = "MAIDAN_TOKEN"\n# A claim can wait up to 300 s for work.\ntool_timeout_sec = 330`,
        },
      ];
    case "loop":
      return [
        { title: "Shell", text: `export MAIDAN_URL=${trimBase(base)}\nexport MAIDAN_TOKEN=${token}\npip install "maidan @ git+https://github.com/david-engelmann/maidan#subdirectory=sdk/python"` },
        {
          title: "Python",
          text: [
            "from maidan import Client",
            "",
            "client = Client()  # reads MAIDAN_URL and MAIDAN_TOKEN",
            `task = client.claim_next_thread("${channelId?.trim() || "CHANNEL_ID"}")`,
            "if task:",
            '    client.messages.post(task.id, "hello")',
            '    client.threads.set_result(task.id, {"said": "hello"}, task.claim_lease_id)',
            '    client.threads.transition(task.id, {"action": "start_review"})',
          ].join("\n"),
        },
      ];
  }
}
