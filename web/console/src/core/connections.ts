// A workspace's own GitHub, Slack and Linear: the account it connects, and the
// words the console puts on each. The token goes to the server once, is checked
// with the provider there, and is never read back: a row only ever says whether
// one is stored and whose it is. Plain data in, plain data out, so the words
// are tested without a page.
import { apiPath, failure, guarded, headers, writeInit, ApiError, type Credentials, type CredentialMode } from "./api.ts";

export type ConnectionSurface = "github" | "slack" | "linear";

/**
 * Whose credential serves the surface: the workspace's own, the server's, or
 * none. `null` is the server's word for a workspace nothing serves.
 */
export type ConnectionSource = "workspace" | "instance" | null;

/** Where a provider posts to this workspace, and whether it has a secret to sign with. */
export type Inbound = {
  url: string;
  secret_set: boolean;
};

/** One surface, as `GET /workspaces/{wid}/connections` lists it. */
export type Connection = {
  surface: ConnectionSurface;
  source: ConnectionSource;
  /** The provider's name for the account the token belongs to. */
  account: string | null;
  connected_at: string | null;
  /** Null for a surface with nothing inbound. */
  inbound: Inbound | null;
};

export type ConnectionsRead = { allowed: true; value: Connection[] } | { allowed: false };

/** What a connect sends. Only the fields the person filled in. */
export type ConnectInput = {
  token?: string;
  /** GitHub's webhook secret. */
  webhook_secret?: string;
  /** Slack's signing secret. */
  signing_secret?: string;
};

export class ConnectionsError extends ApiError {}

type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

export type SurfaceInfo = {
  id: ConnectionSurface;
  name: string;
  /** What the token field is called. */
  token: string;
  /** The same, as a noun in a sentence. */
  noun: string;
  /** The inbound secret's field, for the surfaces that have one. */
  secret: { field: "webhook_secret" | "signing_secret"; label: string; hint: string } | null;
  /** Said under the inbound URL: where to paste it. */
  paste: string;
};

export const SURFACES: ReadonlyArray<SurfaceInfo> = [
  {
    id: "github",
    name: "GitHub",
    token: "Fine-grained personal access token",
    noun: "fine-grained personal access token",
    secret: { field: "webhook_secret", label: "Webhook secret", hint: "16 to 128 characters with no spaces." },
    paste: "Paste this URL into the repository’s webhook settings as the payload URL, and give the webhook the same secret.",
  },
  {
    id: "slack",
    name: "Slack",
    token: "Bot token",
    noun: "bot token",
    secret: { field: "signing_secret", label: "Signing secret", hint: "The signing secret from the Slack app’s Basic Information page." },
    paste: "Paste this URL into the Slack app’s Event Subscriptions as the request URL.",
  },
  {
    id: "linear",
    name: "Linear",
    token: "API key",
    noun: "API key",
    secret: null,
    paste: "",
  },
];

/** A surface by id. One this console does not know has no entry. */
export function surfaceInfo(id: string): SurfaceInfo | null {
  return SURFACES.find((s) => s.id === id) ?? null;
}

/** A surface's name, or what the server sent for one this console does not know. */
export function surfaceName(id: string): string {
  return surfaceInfo(id)?.name ?? id;
}

/** Said in place of the connections to someone who may not see them. */
export const CONNECTIONS_NEED = "Only a workspace admin sees and sets this workspace’s connections. Ask one to connect it.";

/** Said above the rows. The token is checked once and then held where nobody can read it. */
export const CONNECTIONS_HELP =
  "Connect this workspace’s own GitHub, Slack and Linear accounts. A token is checked with its provider when you save it, then stored encrypted and never shown again. A workspace’s own connection is always the one it uses.";

/** The date a connection was made, in the reader's own format. */
export function connectedOn(iso: string, format: (d: Date) => string = (d) => d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" })): string | null {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? null : format(at);
}

/** Which of the three states a row is in, as one chip word. */
export function stateWord(source: ConnectionSource): string {
  if (source === "workspace") return "Connected";
  if (source === "instance") return "Server’s account";
  return "Not connected";
}

/** The row's state in a plain sentence. */
export function stateLine(c: Pick<Connection, "surface" | "source" | "account" | "connected_at">, format?: (d: Date) => string): string {
  const name = surfaceName(c.surface);
  if (c.source === "workspace") {
    const day = c.connected_at ? connectedOn(c.connected_at, format) : null;
    const who = c.account ? `Connected with this workspace’s own ${name} account, ${c.account}` : `Connected with this workspace’s own ${name} account`;
    return day ? `${who}, since ${day}.` : `${who}.`;
  }
  if (c.source === "instance") return `Using this server’s own ${name} connection. Connect this workspace’s own account to use that instead.`;
  return `Not connected. Nothing in this workspace can send to or read from ${name} until it is connected.`;
}

/**
 * The URL to show and copy. A server that knows no public origin answers a
 * path (`/workspaces/…`), which a provider cannot post to, so the browser's
 * own origin is put in front of it.
 */
export function absoluteInboundUrl(url: string, origin: string): string {
  return url.startsWith("/") ? `${origin.replace(/\/+$/, "")}${url}` : url;
}

/** What the inbound secret's state means for the URL above it. */
export function inboundLine(surface: ConnectionSurface, inbound: Inbound): string {
  const secret = surface === "slack" ? "signing secret" : "webhook secret";
  return inbound.secret_set
    ? `A ${secret} is set. Deliveries to this URL are checked against it.`
    : `No ${secret} is set, so this URL accepts nothing yet. Add one below.`;
}

// The same shape the server holds a GitHub webhook secret to, so a typo is
// caught here and the server keeps the last word.
const WEBHOOK_SECRET = /^\S{16,128}$/;

/**
 * Why this surface's form cannot be sent yet, or null when it can. A surface
 * that already has its own connection may send a secret alone and keep the
 * token it holds.
 */
export function connectRefusal(surface: ConnectionSurface, input: ConnectInput, own: boolean): string | null {
  const token = input.token?.trim() ?? "";
  const info = surfaceInfo(surface);
  const secret = info?.secret ? (input[info.secret.field]?.trim() ?? "") : "";
  if (!token && !(own && secret)) return `Paste the ${(info?.noun ?? "token")} to connect ${surfaceName(surface)}.`;
  if (surface === "github" && secret && !WEBHOOK_SECRET.test(secret)) return "A webhook secret is 16 to 128 characters with no spaces.";
  return null;
}

/** The request body: trimmed, with nothing the person left empty. */
export function connectBody(surface: ConnectionSurface, input: ConnectInput): ConnectInput {
  const body: ConnectInput = {};
  const token = input.token?.trim();
  if (token) body.token = token;
  const field = surfaceInfo(surface)?.secret?.field;
  const secret = field ? input[field]?.trim() : "";
  if (field && secret) body[field] = secret;
  return body;
}

/**
 * A server's sentence with anything the person typed taken out of it, so a
 * provider that quotes the token in its refusal never puts it on the page.
 */
export function scrub(text: string, ...typed: string[]): string {
  let out = text;
  for (const secret of typed.map((t) => t.trim()).filter((t) => t.length >= 4)) out = out.split(secret).join("[hidden]");
  return out;
}

export class ConnectionsApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = guarded(fetchFn, ConnectionsError);
    this.creds = creds;
    this.mode = mode;
  }

  /** The three surfaces. A credential that is not a workspace admin's is `allowed: false`, not an error. */
  async list(workspace: string): Promise<ConnectionsRead> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspace}/connections`), { headers: headers(c), credentials: "include" });
    if (res.status === 403) return { allowed: false };
    if (!res.ok) return failure(res, this.mode(), ConnectionsError);
    return { allowed: true, value: (await res.json()) as Connection[] };
  }

  /**
   * Connect a surface, or replace what it holds. The server asks the provider
   * to accept the token first, and says why in its own words when it does not.
   */
  async connect(workspace: string, surface: ConnectionSurface, input: ConnectInput): Promise<Connection> {
    const c = this.creds();
    const body = JSON.stringify(connectBody(surface, input));
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspace}/connections/${surface}`), writeInit({ method: "PUT", headers: headers(c, true), credentials: "include", body }));
    // A refused token (400) and a server that cannot keep one (503) say why and what to set.
    if (!res.ok) return failure(res, this.mode(), ConnectionsError, (status, detail) => ((status === 400 || status === 503) && detail ? detail : null));
    return (await res.json()) as Connection;
  }

  /** Remove the workspace's own connection. The server answers the same whether or not there was one. */
  async remove(workspace: string, surface: ConnectionSurface): Promise<void> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspace}/connections/${surface}`), writeInit({ method: "DELETE", headers: headers(c), credentials: "include" }));
    if (!res.ok) return failure(res, this.mode(), ConnectionsError);
  }
}
