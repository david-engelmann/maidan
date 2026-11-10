// People and Swarm: the roster with presence, a member's details, the
// workspace's footprint lists, and the swarm builder's role circuit.
// Everything here is plain data in, plain data out, so it can be tested.
import { handleFrom, nameRefusal } from "./addAgent.ts";
import { apiPath, bearerRefusal, hasToken, headers, humanError, noteAuthLost, trimBase, writeInit, type CredentialMode, type Credentials } from "./api.ts";
import type { Fetch, Thread } from "./board.ts";
import type { ControlFrame } from "./live.ts";

export type Person = { id: string; name: string; handle: string; kind: "agent" | "human" | string; createdAt: string | null };
export type Presence = "online" | "away" | "offline";
export type Token = { id: string; label: string | null; capabilities: string[]; createdAt: string; expiresAt: string | null; revokedAt: string | null };
export type Meter = { meter: string; quantity: number; allowance: number | null; over: boolean };
export type Usage = { month: string; meters: Meter[] };
export type Claim = { assignee_id: string; repo: string; mode: string; lease_expires_at: string | null; thread_id: string; items: Array<{ kind: string; value: string }> };
export type Deferred = { thread_id: string; title?: string | null; reasons: string[] };
export type Slot = { slot: string; in_use: number; capacity: number | null };
export type Pause = { repo: string; paused_by: string; paused_at: string; reason?: string | null };

export const METER_LABELS: Record<string, string> = {
  requests: "Requests",
  mcp_calls: "MCP tool calls",
  embedding_calls: "Embeddings",
  deliveries: "Deliveries sent",
  wasi_ms: "WASI run time (ms)",
  stored_bytes: "Stored bytes",
};

export const REASON_LABELS: Record<string, string> = {
  overlap: "A live claim overlaps its footprint",
  repo_paused: "Its repository is paused",
  slot_full: "A slot it needs is at capacity",
};

export const WORKER_PRESET = "maidan.agent.worker";
export const CAPS_REQUIRED = "Name at least one capability, or use the worker preset.";

export type InviteRole = "viewer" | "member";
export type Invite = { id: string; email: string; role: InviteRole | string; createdAt: string; expiresAt: string; revokedAt: string | null; acceptedAt: string | null };
export const EMAIL_REQUIRED = "Enter the email address the person signs in with.";

/** Why an email cannot be invited, or null when it can. The server checks again. */
export function emailRefusal(email: string): string | null {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()) ? null : EMAIL_REQUIRED;
}

/** An invite's standing in words. */
export function inviteState(i: Invite, now: number): "accepted" | "revoked" | "expired" | "pending" {
  if (i.acceptedAt) return "accepted";
  if (i.revokedAt) return "revoked";
  if (Date.parse(i.expiresAt) <= now) return "expired";
  return "pending";
}

/** The full link to send: the server returns a path, so join it to the API origin. */
export function inviteLink(base: string, acceptPath: string): string {
  return `${trimBase(base)}${acceptPath}`;
}

export const REPO_REQUIRED = "Name the repository to pause, as owner/name.";

/** Why a repository name cannot be paused, or null when it can. */
export function repoRefusal(repo: string): string | null {
  return /^[\w.-]+\/[\w.-]+$/.test(repo.trim()) ? null : REPO_REQUIRED;
}

// ---- Presence ------------------------------------------------------------

/** The roster after one control frame: a snapshot replaces, an update edits. */
export function applyPresence(prev: ReadonlyMap<string, Presence>, frame: ControlFrame): Map<string, Presence> | null {
  if (frame.type === "presence_snapshot") {
    const next = new Map<string, Presence>();
    const rows = Array.isArray(frame.members) ? (frame.members as Array<{ member_id?: unknown; status?: unknown }>) : [];
    for (const r of rows) if (typeof r.member_id === "string") next.set(r.member_id, asPresence(r.status));
    return next;
  }
  if (frame.type === "presence" && typeof frame.member_id === "string") {
    const next = new Map(prev);
    const s = asPresence(frame.status);
    if (s === "offline") next.delete(frame.member_id);
    else next.set(frame.member_id, s);
    return next;
  }
  return null;
}

function asPresence(v: unknown): Presence {
  return v === "away" ? "away" : v === "offline" ? "offline" : "online";
}

export type ActivityThread = Thread & {
  author_id?: string | null;
  created_by?: string | null;
  created_at?: string | null;
};

function later(prev: string | undefined, at: string | null | undefined): string | undefined {
  if (!at || Number.isNaN(Date.parse(at))) return prev;
  if (!prev || at > prev) return at;
  return prev;
}

/**
 * The latest moment each member shows up on a task: holding it, or having
 * filed it (`created_by` / `author_id`).
 */
export function lastActiveByMember(threads: readonly ActivityThread[]): Map<string, string> {
  const out = new Map<string, string>();
  const note = (id: string | null | undefined, at: string | null | undefined) => {
    if (!id) return;
    const next = later(out.get(id), at);
    if (next) out.set(id, next);
  };
  for (const t of threads) {
    const moved = t.updated_at ?? t.last_progress_at ?? t.work_started_at ?? t.created_at ?? null;
    note(t.assignee_id, moved);
    note(t.created_by, t.created_at ?? t.updated_at ?? null);
    note(t.author_id, t.created_at ?? t.updated_at ?? null);
  }
  return out;
}

/** Online and away win over a timestamp. Otherwise the time, or a quiet empty. */
export function lastActiveFact(presence: Presence, iso: string | null | undefined): { text: string | null; iso: string | null } {
  if (presence === "online") return { text: "Online now", iso: null };
  if (presence === "away") return { text: "Away", iso: null };
  if (iso) return { text: null, iso };
  return { text: "No activity yet", iso: null };
}

const DONE_STATES = new Set(["closed", "archived"]);

/** The clock a piece of work is sorted and labelled by. */
export function workStamp(th: Thread): string {
  return th.last_progress_at ?? th.updated_at ?? th.work_started_at ?? "";
}

/** What a member is on now, and the latest tasks they finished. */
export function workOf<T extends Thread>(id: string, threads: readonly T[]): { now: T[]; recent: T[] } {
  const mine = threads.filter((t) => t.assignee_id === id).sort((a, b) => workStamp(b).localeCompare(workStamp(a)));
  return { now: mine.filter((t) => !DONE_STATES.has(t.state)), recent: mine.filter((t) => DONE_STATES.has(t.state)).slice(0, 5) };
}

export type NamedChannel = { id: string; name: string };

/** Threads from every channel, first copy kept, each tagged with its channel's name. */
export function mergeChannelThreads<T extends { id: string }>(groups: ReadonlyArray<{ channel: NamedChannel; threads: readonly T[] }>): Array<T & { channelName: string }> {
  const seen = new Set<string>();
  const out: Array<T & { channelName: string }> = [];
  for (const g of groups) {
    for (const t of g.threads) {
      if (seen.has(t.id)) continue;
      seen.add(t.id);
      out.push({ ...t, channelName: g.channel.name });
    }
  }
  return out;
}

/** `#name` when the workspace has more than one channel and this task's channel is known. */
export function channelTag(name: string | null | undefined, channelCount: number): string {
  const n = (name ?? "").trim();
  if (!n || channelCount < 2) return "";
  return ` · #${n}`;
}

/** The roster a review starts from. No workspace means there is nobody to load. */
export function freshRoster(workspaceId: string | null): { workspaceId: string | null; ready: boolean } {
  return { workspaceId, ready: !workspaceId };
}

/** A message author: a placeholder until the roster arrives, then the name or a fallback. */
export function authorLabel(ready: boolean, name: string | null | undefined, fallback = "Someone"): string {
  if (name) return name;
  return ready ? fallback : "…";
}

/** What a member holds now: their running task first, else the latest open one. */
export function holding(threads: Thread[]): Map<string, Thread> {
  const out = new Map<string, Thread>();
  const at = (t: Thread) => Date.parse(t.updated_at ?? "") || 0;
  for (const t of threads) {
    if (!t.assignee_id || t.state === "closed" || t.state === "archived") continue;
    const cur = out.get(t.assignee_id);
    const run = (x: Thread) => Boolean(x.work_started_at) && x.state !== "in_review";
    if (!cur || (run(t) && !run(cur)) || (run(t) === run(cur) && at(t) > at(cur))) out.set(t.assignee_id, t);
  }
  return out;
}

export type RosterRow = { person: Person; presence: Presence; task: Thread | null; me: boolean };

/** Online first, then away, then the rest; agents before humans; then by name. */
export function roster(people: Person[], presence: ReadonlyMap<string, Presence>, held: ReadonlyMap<string, Thread>, me: string | null): RosterRow[] {
  const rank: Record<Presence, number> = { online: 0, away: 1, offline: 2 };
  return people
    .map((p) => ({ person: p, presence: presence.get(p.id) ?? ("offline" as Presence), task: held.get(p.id) ?? null, me: p.id === me }))
    .sort((a, b) => rank[a.presence] - rank[b.presence] || (a.person.kind === b.person.kind ? 0 : a.person.kind === "agent" ? -1 : 1) || a.person.name.localeCompare(b.person.name));
}

export function filterRoster(rows: RosterRow[], query: string): RosterRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((r) => r.person.name.toLowerCase().includes(q) || r.person.handle.toLowerCase().includes(q));
}

/** A token's standing in words. */
export function tokenState(t: Token, now: number): "revoked" | "expired" | "active" {
  if (t.revokedAt) return "revoked";
  if (t.expiresAt && Date.parse(t.expiresAt) <= now) return "expired";
  return "active";
}

// ---- The swarm builder ---------------------------------------------------
// Four roles close the loop, in order: a planner files work, a coder takes
// it, a reviewer checks it, a human signs off. The builder seats members in
// those wells. It is a plan kept in this browser; it grants no one anything.

export const ROLES = ["planner", "coder", "reviewer", "human"] as const;
export type Role = (typeof ROLES)[number];
export const ROLE_NAME: Record<Role, string> = { planner: "Planner", coder: "Coder", reviewer: "Reviewer", human: "Human" };
export type Seats = Record<Role, string | null>;
export const NO_SEATS: Seats = { planner: null, coder: null, reviewer: null, human: null };

/** Why a member cannot take a role, or null when they can. */
export function seatRefusal(seats: Seats, role: Role, person: Person | undefined): string | null {
  if (!person) return "That member is not in this workspace.";
  if (role === "human" && person.kind !== "human") return `${person.name} is an agent. The sign-off seat needs a human.`;
  if (role === "reviewer" && seats.coder === person.id) return `${person.name} is the coder. A reviewer cannot review their own work.`;
  if (role === "coder" && seats.reviewer === person.id) return `${person.name} is the reviewer. A coder cannot review their own work.`;
  return null;
}

/** Seat a member; they leave any seat they held, so one member fills one role. */
export function seat(seats: Seats, role: Role, id: string): Seats {
  const next = { ...seats };
  for (const r of ROLES) if (next[r] === id) next[r] = null;
  next[role] = id;
  return next;
}

export function unseat(seats: Seats, role: Role): Seats {
  return { ...seats, [role]: null };
}

export type Circuit = { placed: number; chain: Role[]; broken: Role | null; complete: boolean; jammed: boolean };

export function circuit(seats: Seats): Circuit {
  const chain: Role[] = [];
  let broken: Role | null = null;
  for (const r of ROLES) {
    if (!seats[r]) {
      broken = r;
      break;
    }
    chain.push(r);
  }
  const placed = ROLES.filter((r) => seats[r]).length;
  return { placed, chain, broken, complete: broken === null, jammed: broken !== null && chain.length > 0 && placed > chain.length };
}

/** The status line under the tile, in the landing's voice. */
export function circuitStatus(c: Circuit, presence: (role: Role) => Presence): string {
  if (c.placed === 0) return "Seat a Planner in the steel. Drag a member, or pick one and choose a well.";
  if (c.complete) {
    const away = ROLES.filter((r) => presence(r) !== "online");
    return away.length ? `Circuit closed. ${away.map((r) => ROLE_NAME[r]).join(", ")} ${away.length === 1 ? "is" : "are"} not online now.` : "Circuit closed. Every seat is online.";
  }
  const broken = c.broken as Role;
  if (c.jammed) return `Jammed. No ${ROLE_NAME[broken]}, so nothing gets past the ${ROLE_NAME[c.chain[c.chain.length - 1]]}.`;
  return `${ROLE_NAME[broken]} next. The channel is cut, waiting.`;
}

const SEATS_KEY = "maidan_console_swarm_seats";

export function loadSeats(storage: Pick<Storage, "getItem">, workspace: string, people: Person[]): Seats {
  try {
    const raw = JSON.parse(storage.getItem(`${SEATS_KEY}:${workspace}`) ?? "null") as Partial<Seats> | null;
    const ids = new Set(people.map((p) => p.id));
    const out: Seats = { ...NO_SEATS };
    for (const r of ROLES) {
      const v = raw?.[r];
      if (typeof v === "string" && ids.has(v)) out[r] = v;
    }
    return out;
  } catch {
    return { ...NO_SEATS };
  }
}

export function saveSeats(storage: Pick<Storage, "setItem">, workspace: string, seats: Seats): void {
  storage.setItem(`${SEATS_KEY}:${workspace}`, JSON.stringify(seats));
}

/** The next well for arrow keys on the 2×2 tile: left/right in a row, up/down in a column. */
export function nextWell(role: Role, key: string): Role {
  const i = ROLES.indexOf(role);
  const col = i % 2;
  const row = Math.floor(i / 2);
  if (key === "ArrowRight") return ROLES[row * 2 + ((col + 1) % 2)];
  if (key === "ArrowLeft") return ROLES[row * 2 + ((col + 1) % 2)];
  if (key === "ArrowDown") return ROLES[((row + 1) % 2) * 2 + col];
  if (key === "ArrowUp") return ROLES[((row + 1) % 2) * 2 + col];
  if (key === "Home") return ROLES[0];
  if (key === "End") return ROLES[3];
  return role;
}

// ---- Reads and writes ----------------------------------------------------

export class PeopleError extends Error {}

/** A refused add: only a workspace admin adds agents, from their own sign-in or an admin token. */
export const ADD_AGENT_REFUSED = "Only a workspace admin can add an agent. Ask one to add it for you.";

async function fail(res: Response, mode: CredentialMode | null): Promise<never> {
  noteAuthLost(res);
  let detail: string | null = null;
  try {
    const body = (await res.json()) as { error?: string; message?: string; detail?: string };
    detail = body.message ?? body.detail ?? body.error ?? null;
  } catch {
    detail = null;
  }
  if ((res.status === 400 || res.status === 409 || res.status === 422) && detail) throw new PeopleError(detail);
  throw new PeopleError(humanError(res.status, detail, mode));
}

export class PeopleApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = fetchFn;
    this.creds = creds;
    this.mode = mode;
  }

  private async read<T>(url: string): Promise<T> {
    const c = this.creds();
    const res = await this.fetchFn(url, { headers: headers(c), credentials: "include" });
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as T;
  }

  /** A read the session proxy also serves. */
  private get<T>(path: string): Promise<T> {
    return this.read<T>(apiPath(this.creds(), path));
  }

  /** A read only the bearer tree serves; without a token it is refused here. */
  private bearer<T>(path: string): Promise<T> {
    const c = this.creds();
    const why = bearerRefusal(c);
    if (why) return Promise.reject(new PeopleError(why));
    return this.read<T>(`${trimBase(c.base)}${path}`);
  }

  async people(workspace: string): Promise<Person[]> {
    const rows = await this.get<Array<{ id: string; display_name?: string | null; handle?: string; kind?: string; created_at?: string; tombstoned_at?: string | null }>>(`/workspaces/${workspace}/members`);
    return rows
      .filter((m) => !m.tombstoned_at)
      .map((m) => ({ id: m.id, name: m.display_name || m.handle || m.id.slice(0, 8), handle: m.handle ?? "", kind: m.kind ?? "", createdAt: m.created_at ?? null }));
  }

  async tokens(workspace: string, member: string): Promise<Token[]> {
    const rows = await this.get<Array<{ id: string; label?: string | null; capabilities?: string[]; created_at: string; expires_at?: string | null; revoked_at?: string | null }>>(`/workspaces/${workspace}/members/${member}/tokens`);
    return rows.map((t) => ({ id: t.id, label: t.label ?? null, capabilities: t.capabilities ?? [], createdAt: t.created_at, expiresAt: t.expires_at ?? null, revokedAt: t.revoked_at ?? null }));
  }

  usage(workspace: string): Promise<Usage> {
    return this.get<Usage>(`/workspaces/${workspace}/usage-meters`);
  }

  claims(workspace: string): Promise<Claim[]> {
    return this.bearer<Claim[]>(`/workspaces/${workspace}/claims`);
  }

  deferred(workspace: string): Promise<Deferred[]> {
    return this.bearer<Deferred[]>(`/workspaces/${workspace}/deferred`);
  }

  slots(workspace: string): Promise<Slot[]> {
    return this.bearer<Slot[]>(`/workspaces/${workspace}/slots`);
  }

  pauses(workspace: string): Promise<Pause[]> {
    return this.bearer<Pause[]>(`/workspaces/${workspace}/repo-pauses`);
  }

  async pause(workspace: string, repo: string, reason: string): Promise<void> {
    const why = repoRefusal(repo);
    if (why) throw new PeopleError(why);
    await this.write("PUT", `/workspaces/${workspace}/repo-pauses`, { repo: repo.trim(), reason: reason.trim() || undefined });
  }

  async resume(workspace: string, repo: string): Promise<void> {
    await this.write("DELETE", `/workspaces/${workspace}/repo-pauses?repo=${encodeURIComponent(repo)}`, undefined);
  }

  /**
   * Mints a token for a member. The secret comes back once; the caller shows
   * it and drops it, and nothing here keeps or logs it. Needs a bearer token.
   */
  async mint(workspace: string, member: string, req: { label: string; capabilities: string[]; preset: string | null }): Promise<{ id: string; secret: string }> {
    if (!req.preset && !req.capabilities.length) throw new PeopleError(CAPS_REQUIRED);
    const body = req.preset ? { label: req.label.trim() || null, capability_set: req.preset, capabilities: [] } : { label: req.label.trim() || null, capabilities: req.capabilities };
    return this.secretWrite("POST", `/workspaces/${workspace}/members/${member}/tokens`, body);
  }

  /**
   * Adds an agent: a member and its worker token in one call. A signed-in
   * admin goes through the session route, a token through the bearer one;
   * both mint `maidan.agent.worker` and nothing more. The secret comes back
   * once; nothing here keeps or logs it.
   */
  async addAgent(workspace: string, name: string): Promise<{ memberId: string; handle: string; secret: string }> {
    const why = nameRefusal(name);
    if (why) throw new PeopleError(why);
    const c = this.creds();
    if (!hasToken(c) && !c.sessionMemberId) throw new PeopleError("Sign in first.");
    const body = { handle: handleFrom(name), display_name: name.trim() };
    const init: RequestInit = { headers: headers(c, true), credentials: "include", body: JSON.stringify(body) };
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspace}/agents`), writeInit({ ...init, method: "POST" }));
    if (res.status === 403) {
      noteAuthLost(res);
      throw new PeopleError(ADD_AGENT_REFUSED);
    }
    if (!res.ok) return fail(res, this.mode());
    const out = (await res.json()) as { member?: { id?: string; handle?: string }; token?: { secret?: string } };
    if (typeof out.member?.id !== "string" || typeof out.token?.secret !== "string") throw new PeopleError("The server did not send the new token back.");
    return { memberId: out.member.id, handle: out.member.handle ?? body.handle, secret: out.token.secret };
  }

  /** Rotates a token: the old secret stops working and the new one comes back once. */
  rotate(id: string): Promise<{ id: string; secret: string }> {
    return this.secretWrite("POST", `/tokens/${encodeURIComponent(id)}/rotate`, undefined);
  }

  async revoke(id: string): Promise<void> {
    await this.write("DELETE", `/tokens/${encodeURIComponent(id)}`, undefined);
  }

  /**
   * The workspace's invites, or `null` when the server refuses this person
   * the list (403): only a workspace admin invites, so the panel hides for
   * everyone else instead of greeting them with an error.
   */
  async invites(workspace: string): Promise<Invite[] | null> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspace}/invites`), { headers: headers(c), credentials: "include" });
    if (res.status === 403) return null;
    if (!res.ok) return fail(res, this.mode());
    const rows = (await res.json()) as Array<{ id: string; email: string; role: string; created_at: string; expires_at: string; revoked_at?: string | null; accepted_at?: string | null }>;
    return rows.map((i) => ({ id: i.id, email: i.email, role: i.role, createdAt: i.created_at, expiresAt: i.expires_at, revokedAt: i.revoked_at ?? null, acceptedAt: i.accepted_at ?? null }));
  }

  /**
   * Invites a person. The link comes back once; the caller shows it and
   * drops it, and nothing here keeps or logs it. A signed-in admin goes
   * through the session route, a token through the bearer one.
   */
  async invite(workspace: string, req: { email: string; role: InviteRole }): Promise<{ id: string; link: string }> {
    const why = emailRefusal(req.email);
    if (why) throw new PeopleError(why);
    const res = await this.either("POST", `/workspaces/${workspace}/invites`, { email: req.email.trim(), role: req.role });
    const out = (await res.json()) as { invite?: { id?: string }; accept_path?: string };
    if (typeof out.invite?.id !== "string" || typeof out.accept_path !== "string") throw new PeopleError("The server did not send the invite link back.");
    return { id: out.invite.id, link: inviteLink(this.creds().base, out.accept_path) };
  }

  async revokeInvite(workspace: string, id: string): Promise<void> {
    await this.either("DELETE", `/workspaces/${workspace}/invites/${encodeURIComponent(id)}`, undefined);
  }

  private async secretWrite(method: string, path: string, body: unknown): Promise<{ id: string; secret: string }> {
    const res = await this.raw(method, path, body);
    const out = (await res.json()) as { id?: string; secret?: string };
    if (typeof out.id !== "string" || typeof out.secret !== "string") throw new PeopleError("The server did not send the new token back.");
    return { id: out.id, secret: out.secret };
  }

  private async write(method: string, path: string, body: unknown): Promise<void> {
    await this.raw(method, path, body);
  }

  /** A write the session routes also serve: a token's goes to the bearer route, a signed-in person's to the session route. */
  private async either(method: string, path: string, body: unknown): Promise<Response> {
    const c = this.creds();
    if (!hasToken(c) && !c.sessionMemberId) throw new PeopleError("Sign in first.");
    const init: RequestInit = { method, headers: headers(c, body !== undefined), credentials: "include" };
    if (body !== undefined) init.body = JSON.stringify(body);
    const res = await this.fetchFn(apiPath(c, path), writeInit(init));
    if (!res.ok) return fail(res, this.mode());
    return res;
  }

  private async raw(method: string, path: string, body: unknown): Promise<Response> {
    const c = this.creds();
    const why = bearerRefusal(c);
    if (why) throw new PeopleError(why);
    const init: RequestInit = { method, headers: headers(c, body !== undefined), credentials: "include" };
    if (body !== undefined) init.body = JSON.stringify(body);
    const res = await this.fetchFn(`${trimBase(c.base)}${path}`, writeInit(init));
    if (!res.ok) return fail(res, this.mode());
    return res;
  }
}
