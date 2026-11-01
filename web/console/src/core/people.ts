// People and Swarm: the roster with presence, a member's details, the
// workspace's footprint lists, and the swarm builder's role circuit.
// Everything here is plain data in, plain data out, so it can be tested.
import { apiPath, bearerRefusal, headers, humanError, trimBase, writeInit, type CredentialMode, type Credentials } from "./api.ts";
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

async function fail(res: Response, mode: CredentialMode | null): Promise<never> {
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

  /** Rotates a token: the old secret stops working and the new one comes back once. */
  rotate(id: string): Promise<{ id: string; secret: string }> {
    return this.secretWrite("POST", `/tokens/${encodeURIComponent(id)}/rotate`, undefined);
  }

  async revoke(id: string): Promise<void> {
    await this.write("DELETE", `/tokens/${encodeURIComponent(id)}`, undefined);
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
