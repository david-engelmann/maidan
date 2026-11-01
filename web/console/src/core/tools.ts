// Tools: the operator and self-service reads and writes the classic board
// kept on its Tools tabs, minus the gates Needs you already covers. Plus the
// artifact store. Everything here is plain data in, plain data out.
import { apiPath, bearerRefusal, headers, humanError, trimBase, writeInit, type CredentialMode, type Credentials } from "./api.ts";
import type { Fetch } from "./board.ts";

export type Notification = { id: string; kind: string; thread_id?: string | null; read_at?: string | null; created_at?: string };
export type DeliveryMode = "immediate" | "digest" | string;
export type NotificationPref = { kind: string; muted: boolean };
export type Follow = { channel_id?: string; thread_id?: string };
export type DeliveryStatus = "pending" | "quarantined" | "delivered";
export type Delivery = {
  id: number | string;
  kind: string;
  target_url: string;
  attempts: number;
  last_error?: string | null;
  quarantined_at?: string | null;
  delivered_at?: string | null;
  next_attempt_at?: string | null;
  created_at?: string | null;
};
export type SlashCommand = { id: string; name: string; description?: string | null; handler_kind: string; handler_target: string; enabled: boolean };
export type AuditEvent = { occurred_at: string; action: string; actor_id?: string | null; target_kind?: string | null; target_id?: string | null };
export type ReindexJob = { job_id: string; status: string; workspace_id?: string | null; embedding_model: string; processed?: number; failed?: number; error?: string | null };
export type Depth = { open: number; ready: number; assigned: number; blocked: number; unclaimable: number };
export type Occupancy = { queued: number; claimed: number; working: number; blocked: number };
export type Schedule = { id?: string; title: string; interval_secs?: number | null; active: boolean; next_run_at: string };
export type WorkEvent = { id: number | string; kind: string; thread_id?: string | null; channel_id?: string | null };
export type Peer = { id: string; name?: string | null; base_url?: string | null };
export type ArtifactMeta = { sha256?: string; filename?: string | null; mime_type?: string | null; size_bytes?: number; kind?: string | null; created_at?: string | null };

export type SearchHit = { message_id?: string; id?: string; thread_id?: string | null; body?: string | null; snippet?: string | null };
export type AppInstallation = { id: string; app_id: string; revoked_at?: string | null };
export type SwitchableWorkspace = { workspace_id: string; name: string; current?: boolean };
export const SEARCH_MODES = ["lexical", "semantic"] as const;

/** The query string for a workspace search; empty fields stay out. */
export function searchQuery(q: string, mode: string, channel = "", author = ""): string {
  const p = new URLSearchParams({ q: q.trim(), mode, limit: "20" });
  if (channel.trim()) p.set("channel", channel.trim());
  if (author.trim()) p.set("author", author.trim());
  return p.toString();
}

/** Where switching workspace goes: the sign-in flow, back to the console. */
export function switchHref(base: string, workspaceId: string, loginPath = "/auth/oidc/login"): string {
  return `${trimBase(base)}${loginPath}?workspace_id=${encodeURIComponent(workspaceId)}&return_to=/ui/`;
}

export const HANDLE_REQUIRED = "Give the agent a handle: lower case letters, digits and dashes.";
export function handleRefusal(handle: string): string | null {
  return /^[a-z0-9][a-z0-9-]{0,62}$/.test(handle.trim()) ? null : HANDLE_REQUIRED;
}

export function peerRefusal(name: string, baseUrl: string): string | null {
  if (!name.trim()) return "Name the peer.";
  return /^https?:\/\/\S+$/.test(baseUrl.trim()) ? null : "Give the peer's base URL, starting with https://.";
}

export const DELIVERY_KINDS = ["all", "webhook", "automation"] as const;
export const HANDLER_KINDS = ["http", "mcp_tool"] as const;
export const NOTIFICATION_KINDS = ["message_posted", "mention_recorded", "thread_ready", "thread_result_set", "review_submitted", "claim_expired", "thread_landed"] as const;
export const EVENT_KINDS = ["message_posted", "mention_recorded", "thread_created", "thread_transitioned", "thread_ready", "thread_result_set", "thread_landed", "claim_expired"] as const;

/** The query string the deliveries list reads, by status. */
export function deliveryQuery(kind: string, status: DeliveryStatus, limit = 50): string {
  let q = `kind=${encodeURIComponent(kind)}&limit=${limit}`;
  if (status === "quarantined") q += "&quarantined=true";
  else if (status === "delivered") q += "&delivered=true";
  return q;
}

export function deliveryState(d: Delivery): "dlq" | "delivered" | "pending" {
  if (d.quarantined_at) return "dlq";
  if (d.delivered_at) return "delivered";
  return "pending";
}

export function scheduleCadence(s: Schedule): string {
  const n = s.interval_secs;
  if (!n) return "Runs once";
  if (n % 86400 === 0) return n === 86400 ? "Every day" : `Every ${n / 86400} days`;
  if (n % 3600 === 0) return n === 3600 ? "Every hour" : `Every ${n / 3600} hours`;
  if (n % 60 === 0) return n === 60 ? "Every minute" : `Every ${n / 60} minutes`;
  return `Every ${n} seconds`;
}

export const NAME_REQUIRED = "Name the command, in lower case with no slash.";
export const TARGET_REQUIRED = "Say where the command goes: a URL for HTTP, or a tool name for MCP.";

/** Why a slash command cannot be registered, or null when it can. */
export function slashRefusal(name: string, target: string): string | null {
  if (!/^[a-z0-9][a-z0-9_-]{0,31}$/.test(name.trim().replace(/^\//, ""))) return NAME_REQUIRED;
  if (!target.trim()) return TARGET_REQUIRED;
  return null;
}

export function emailRefusal(email: string): string | null {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()) ? null : "That does not look like an email address.";
}

/** Capabilities typed as a list: commas or spaces, deduped, in order. */
export function parseCaps(raw: string): string[] {
  const out: string[] = [];
  for (const c of raw.split(/[\s,]+/)) if (c && !out.includes(c)) out.push(c);
  return out;
}

/** The asked-for capabilities the caller does not hold; a mint cannot widen a grant. */
export function capsExceeding(asked: string[], held: string[] | null): string[] {
  if (!held) return [];
  const have = new Set(held);
  return asked.filter((c) => !have.has(c));
}

export function formatBytes(n: number | undefined | null): string {
  if (n == null || !Number.isFinite(n)) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function mediaEssence(type: string | null | undefined): string {
  return String(type ?? "").split(";")[0].trim().toLowerCase();
}

const INLINE_IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

/** Raster images preview inline; SVG and the rest only download. */
export function previewable(type: string | null | undefined): boolean {
  return INLINE_IMAGE_TYPES.has(mediaEssence(type));
}

export function isSha(s: string): boolean {
  return /^[0-9a-f]{64}$/.test(s.trim().toLowerCase());
}

// ---- Reads and writes ----------------------------------------------------

export class ToolsError extends Error {}

async function fail(res: Response, mode: CredentialMode | null): Promise<never> {
  let detail: string | null = null;
  try {
    const body = (await res.json()) as { error?: string; message?: string; detail?: string };
    detail = body.message ?? body.detail ?? body.error ?? null;
  } catch {
    detail = null;
  }
  if ((res.status === 400 || res.status === 409 || res.status === 422) && detail) throw new ToolsError(detail);
  throw new ToolsError(humanError(res.status, detail, mode));
}

export class ToolsApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = fetchFn;
    this.creds = creds;
    this.mode = mode;
  }

  private async get<T>(path: string): Promise<T> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, path), { headers: headers(c), credentials: "include" });
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as T;
  }

  /** A read that answers 404 when there is nothing: null, not an error. */
  private async maybe<T>(path: string): Promise<T | null> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, path), { headers: headers(c), credentials: "include" });
    if (res.status === 404) return null;
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as T;
  }

  private async bearerGet<T>(path: string): Promise<T> {
    const c = this.creds();
    const why = bearerRefusal(c);
    if (why) throw new ToolsError(why);
    const res = await this.fetchFn(`${trimBase(c.base)}${path}`, { headers: headers(c), credentials: "include" });
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as T;
  }

  /** The server's VAPID key; `public_key` is null when push is not set up. */
  vapidKey(): Promise<{ public_key?: string | null; reason?: string | null } | null> {
    return this.maybe("/web-push/vapid-public-key");
  }

  /** Store a browser push subscription for this member. */
  async savePush(member: string, sub: { endpoint: string; keys: Record<string, string> }): Promise<void> {
    await this.send("POST", `/members/${member}/push-subscriptions`, sub);
  }

  /** Remove the server copy of a subscription. Bearer only, so it skips quietly for a session. */
  async forgetPush(member: string, endpoint: string): Promise<void> {
    if (bearerRefusal(this.creds())) return;
    const subs = await this.bearerGet<Array<{ id: string; endpoint?: string }>>(`/members/${member}/push-subscriptions`);
    for (const s of subs.filter((x) => x.endpoint === endpoint)) await this.send("DELETE", `/members/${member}/push-subscriptions/${s.id}`, undefined, true);
  }

  private async send<T>(method: string, path: string, body?: unknown, bearerOnly = false): Promise<T | null> {
    const c = this.creds();
    if (bearerOnly) {
      const why = bearerRefusal(c);
      if (why) throw new ToolsError(why);
    }
    const init: RequestInit = { method, headers: headers(c, body !== undefined), credentials: "include" };
    if (body !== undefined) init.body = JSON.stringify(body);
    const url = bearerOnly ? `${trimBase(c.base)}${path}` : apiPath(c, path);
    const res = await this.fetchFn(url, writeInit(init));
    if (!res.ok) return fail(res, this.mode());
    const text = await res.text();
    return text ? (JSON.parse(text) as T) : null;
  }

  /** Erase a file from the store for good. Admin token only. */
  async eraseArtifact(sha: string): Promise<void> {
    await this.send("DELETE", `/artifacts/${sha}`, undefined, true);
  }

  /**
   * Mint the first admin token for this signed-in session. The secret comes
   * back once; the caller shows it with SecretOnce and never logs it.
   */
  async mintAdmin(): Promise<{ secret: string }> {
    const c = this.creds();
    const res = await this.fetchFn(`${trimBase(c.base)}/auth/session/mint`, writeInit({ method: "POST", headers: headers(c, false), credentials: "include" }));
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as { secret: string };
  }

  // Inbox
  notifications(member: string, unreadOnly: boolean): Promise<Notification[]> {
    return this.get(`/members/${member}/notifications?unread_only=${unreadOnly}&limit=50`);
  }
  async unread(member: string): Promise<number> {
    return (await this.get<{ count: number }>(`/members/${member}/notifications/unread-count`)).count;
  }
  async markRead(member: string, id: string): Promise<void> {
    await this.send("POST", `/members/${member}/notifications/${id}/read`);
  }
  async markAllRead(member: string): Promise<void> {
    await this.send("POST", `/members/${member}/notifications/read-all`);
  }

  // Preferences, self only
  async deliveryMode(member: string): Promise<DeliveryMode | null> {
    return (await this.maybe<{ mode: string }>(`/members/${member}/delivery-mode`))?.mode ?? null;
  }
  async setDeliveryMode(member: string, mode: DeliveryMode): Promise<void> {
    await this.send("PUT", `/members/${member}/delivery-mode`, { mode });
  }
  async email(member: string): Promise<string | null> {
    return (await this.maybe<{ email: string }>(`/members/${member}/email`))?.email ?? null;
  }
  async setEmail(member: string, email: string): Promise<void> {
    const why = emailRefusal(email);
    if (why) throw new ToolsError(why);
    await this.send("PUT", `/members/${member}/email`, { email: email.trim() });
  }
  async clearEmail(member: string): Promise<void> {
    await this.send("DELETE", `/members/${member}/email`);
  }
  prefs(member: string): Promise<NotificationPref[]> {
    return this.get(`/members/${member}/notification-prefs`);
  }
  async setMute(member: string, kind: string, muted: boolean): Promise<void> {
    await this.send("PUT", `/members/${member}/notification-prefs`, { kind, muted });
  }
  follows(member: string, what: "channel" | "thread"): Promise<Follow[]> {
    return this.get(`/members/${member}/${what}-follows`);
  }
  async follow(member: string, what: "channel" | "thread", id: string): Promise<void> {
    await this.send("POST", `/members/${member}/${what}-follows`, what === "channel" ? { channel_id: id.trim() } : { thread_id: id.trim() });
  }
  async unfollow(member: string, what: "channel" | "thread", id: string): Promise<void> {
    await this.send("DELETE", `/members/${member}/${what}-follows/${id}`);
  }
  async setDisplayName(member: string, name: string): Promise<string | null> {
    const m = await this.send<{ display_name?: string | null }>("PATCH", `/members/${member}`, { display_name: name });
    return m?.display_name ?? null;
  }

  // Deliveries
  deliveries(workspace: string, kind: string, status: DeliveryStatus): Promise<Delivery[]> {
    return this.get(`/workspaces/${workspace}/deliveries?${deliveryQuery(kind, status)}`);
  }
  async replay(workspace: string, d: Pick<Delivery, "id" | "kind">): Promise<void> {
    await this.send("POST", `/workspaces/${workspace}/deliveries/${d.id}/replay?kind=${encodeURIComponent(d.kind)}`);
  }

  // Slash commands
  commands(workspace: string): Promise<SlashCommand[]> {
    return this.get(`/workspaces/${workspace}/slash-commands`);
  }
  /** Registers a command; the signing secret, when there is one, comes back once. */
  async register(workspace: string, cmd: { name: string; description: string; handlerKind: string; handlerTarget: string }): Promise<{ secret: string | null }> {
    const why = slashRefusal(cmd.name, cmd.handlerTarget);
    if (why) throw new ToolsError(why);
    const out = await this.send<{ secret?: string | null }>("POST", `/workspaces/${workspace}/slash-commands`, {
      name: cmd.name.trim().replace(/^\//, ""),
      description: cmd.description.trim() || null,
      handler_kind: cmd.handlerKind,
      handler_target: cmd.handlerTarget.trim(),
    });
    return { secret: out?.secret ?? null };
  }
  async revokeCommand(workspace: string, id: string): Promise<void> {
    await this.send("DELETE", `/workspaces/${workspace}/slash-commands/${id}`);
  }

  // Operator
  audit(limit: number): Promise<AuditEvent[]> {
    return this.bearerGet(`/operator/audit?limit=${encodeURIComponent(String(limit))}`);
  }
  async reindex(workspace: string | null): Promise<ReindexJob> {
    const job = await this.send<ReindexJob>("POST", `/operator/reindex-embeddings`, workspace ? { workspace_id: workspace } : {}, workspace === null);
    if (!job) throw new ToolsError("The server did not say which job it started.");
    return job;
  }
  reindexJob(id: string): Promise<ReindexJob> {
    return this.get(`/operator/reindex-embeddings/${encodeURIComponent(id.trim())}`);
  }

  // Explore
  channels(workspace: string): Promise<Array<{ id: string; name: string }>> {
    return this.get(`/workspaces/${workspace}/channels`);
  }
  depth(channel: string): Promise<Depth> {
    return this.get(`/channels/${channel}/queue-depth`);
  }
  occupancy(channel: string): Promise<Occupancy | null> {
    return this.maybe(`/channels/${channel}/occupancy`);
  }
  schedules(workspace: string): Promise<Schedule[]> {
    return this.get(`/workspaces/${workspace}/task-schedules`);
  }
  /** The newest workspace events, raw rows, oldest first (the Activity screen). */
  recentEvents(workspace: string): Promise<Array<Record<string, unknown>>> {
    return this.get(`/workspaces/${workspace}/events?limit=200`);
  }
  async events(workspace: string, kind: string): Promise<WorkEvent[]> {
    const rows = await this.get<WorkEvent[]>(`/workspaces/${workspace}/events?limit=200`);
    return rows.filter((e) => !kind || e.kind === kind).slice(-50).reverse();
  }
  peers(workspace: string): Promise<Peer[]> {
    return this.get(`/workspaces/${workspace}/peers`);
  }
  addPeer(workspace: string, name: string, baseUrl: string): Promise<Peer | null> {
    return this.send("POST", `/workspaces/${workspace}/peers`, { name: name.trim(), base_url: baseUrl.trim() }, true);
  }
  async removePeer(workspace: string, id: string): Promise<void> {
    await this.send("DELETE", `/workspaces/${workspace}/peers/${id}`, undefined, true);
  }

  // Workspace
  async renameWorkspace(workspace: string, name: string): Promise<void> {
    await this.send("PATCH", `/workspaces/${workspace}`, { name: name.trim() });
  }
  createChannel(workspace: string, name: string): Promise<{ id: string; name: string } | null> {
    return this.send("POST", `/workspaces/${workspace}/channels`, { name: name.trim(), private: false });
  }
  createThread(channel: string, title: string): Promise<{ id: string } | null> {
    return this.send("POST", `/channels/${channel}/threads`, { title: title.trim() });
  }
  createAgent(workspace: string, handle: string, display: string): Promise<{ id?: string; member_id?: string } | null> {
    return this.send("POST", `/workspaces/${workspace}/agents`, { handle: handle.trim(), display_name: display.trim() || null }, true);
  }
  workspaceAudit(workspace: string, limit: number): Promise<AuditEvent[]> {
    return this.get(`/workspaces/${workspace}/audit?limit=${limit}`);
  }
  appInstallations(workspace: string): Promise<AppInstallation[]> {
    return this.get(`/workspaces/${workspace}/app-installations`);
  }
  search(workspace: string, query: string): Promise<SearchHit[]> {
    return this.get(`/workspaces/${workspace}/search?${query}`);
  }
  /** Workspaces this browser session can switch to; empty when the server has none to offer. */
  async switchable(): Promise<SwitchableWorkspace[]> {
    const c = this.creds();
    try {
      const res = await this.fetchFn(`${trimBase(c.base)}/auth/session/workspaces`, { credentials: "include" });
      if (!res.ok) return [];
      const body = (await res.json()) as { workspaces?: SwitchableWorkspace[] };
      return Array.isArray(body.workspaces) ? body.workspaces : [];
    } catch {
      return [];
    }
  }

  async me(): Promise<string[]> {
    return (await this.get<{ capabilities?: string[] }>("/me")).capabilities ?? [];
  }

  // Artifacts
  artifact(sha: string): Promise<ArtifactMeta | null> {
    return this.maybe(`/artifacts/${encodeURIComponent(sha.trim().toLowerCase())}/meta`);
  }
  async artifactBlob(sha: string): Promise<Blob> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/artifacts/${encodeURIComponent(sha)}`), { headers: headers(c), credentials: "include" });
    if (!res.ok) return fail(res, this.mode());
    return res.blob();
  }
  async upload(file: Blob & { name?: string }, kind = "attachment"): Promise<{ sha256: string }> {
    const c = this.creds();
    const params = new URLSearchParams({ kind });
    if (file.type) params.set("mime_type", file.type);
    if (file.name) params.set("filename", file.name);
    const res = await this.fetchFn(apiPath(c, `/artifacts?${params}`), writeInit({ method: "POST", headers: headers(c), credentials: "include", body: file }));
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as { sha256: string };
  }
  async linked(thread: string): Promise<string[]> {
    const rows = await this.get<Array<{ sha256?: string }>>(`/threads/${thread}/artifacts`);
    return rows.map((r) => String(r.sha256 ?? "")).filter(Boolean);
  }
  async link(thread: string, sha: string): Promise<void> {
    await this.send("PUT", `/threads/${thread}/artifacts/${encodeURIComponent(sha)}`);
  }
  async unlink(thread: string, sha: string): Promise<void> {
    await this.send("DELETE", `/threads/${thread}/artifacts/${encodeURIComponent(sha)}`);
  }
}
