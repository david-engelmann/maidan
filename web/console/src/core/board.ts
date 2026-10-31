// The board's model, a port of the classic board.js without the DOM: which
// lane a task sits in, what its card says, who closed it, and the reads and
// writes behind the lanes. Fetch and the clock are passed in, so it is tested.
import { apiPath, bearerRefusal, headers, uiPath, writeInit, humanError, type Credentials, type CredentialMode } from "./api.ts";

export { bearerRefusal } from "./api.ts";

export type LaneKey = "open" | "working" | "review" | "done";

export const LANES: ReadonlyArray<{ key: LaneKey; title: string; short: string }> = [
  { key: "open", title: "Open", short: "Open" },
  { key: "working", title: "In progress", short: "Working" },
  { key: "review", title: "Needs review", short: "Review" },
  { key: "done", title: "Done", short: "Done" },
];

export type Thread = {
  id: string;
  title?: string | null;
  state: string;
  assignee_id?: string | null;
  work_started_at?: string | null;
  assignment_expires_at?: string | null;
  updated_at?: string | null;
  closed_without_review?: boolean;
  stalled?: boolean;
  last_progress_at?: string | null;
  status?: { status: string; note?: string | null } | null;
  block?: { reason: string; note?: string | null } | null;
  human_wait?: { state: string; since?: string; returns_at?: string } | null;
};

export type Gate = { hasSchema: boolean };
export type Channel = { id: string; name: string };
export type Member = { id: string; name: string; kind?: string };
export type Transition = { actor_id?: string | null; to_state?: string; from_state?: string; at?: string; created_at?: string };

export type Chrome = { label: string; hint: string; lane: LaneKey };

/** Where a task sits and what its card calls it. First match wins, as on /ui. */
export function chromeFor(th: Thread, gate?: Gate | null): Chrome {
  if (th.state === "closed" || th.state === "archived") return { label: "done", hint: "thread is closed", lane: "done" };
  if (gate)
    return gate.hasSchema
      ? { label: "needs-input", hint: "waiting on structured input", lane: "review" }
      : { label: "needs-approval", hint: "waiting on human approval", lane: "review" };
  if (th.state === "in_review") return { label: "in review", hint: "result posted, waiting on review", lane: "review" };
  if (th.assignee_id && th.work_started_at) return { label: "running", hint: "claimed and working", lane: "working" };
  if (th.assignee_id) return { label: "claimed", hint: "claimed, not started yet", lane: "working" };
  return { label: "open", hint: "unclaimed, free to take", lane: "open" };
}

/** The state word a card shows: a close with no approval says so. */
export function stateLabel(th: Thread, chrome: Chrome): string {
  return th.closed_without_review ? "closed without review" : chrome.label;
}

export type Card = { thread: Thread; chrome: Chrome; title: string; label: string };

/** Every task in its lane, newest change first, lanes in board order. */
export function toLanes(threads: Thread[], gates: Record<string, Gate>): Record<LaneKey, Card[]> {
  const out: Record<LaneKey, Card[]> = { open: [], working: [], review: [], done: [] };
  for (const th of threads) {
    const chrome = chromeFor(th, gates[th.id]);
    out[chrome.lane].push({ thread: th, chrome, title: th.title || th.id.slice(0, 8), label: stateLabel(th, chrome) });
  }
  const t = (c: Card) => Date.parse(c.thread.updated_at ?? "") || 0;
  for (const k of Object.keys(out) as LaneKey[]) out[k].sort((a, b) => t(b) - t(a));
  return out;
}

export function ago(ts: string | null | undefined, now: number): string {
  if (!ts) return "";
  const at = Date.parse(ts);
  if (Number.isNaN(at)) return "";
  const secs = Math.max(0, Math.round((now - at) / 1000));
  if (secs < 45) return "just now";
  if (secs < 3600) return `${Math.round(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.round(secs / 3600)}h ago`;
  return `${Math.round(secs / 86400)}d ago`;
}

export function leaseLeft(ts: string | null | undefined, now: number): string {
  if (!ts) return "";
  const at = Date.parse(ts);
  if (Number.isNaN(at)) return "";
  const secs = Math.round((at - now) / 1000);
  if (secs <= 0) return "lease lapsed";
  return secs < 120 ? `lease ${secs}s` : `lease ${Math.round(secs / 60)}m`;
}

/** The card's time: the lease while it is being worked, else its last change. */
export function cardWhen(card: Card, now: number): string {
  return card.chrome.lane === "working" && card.thread.assignment_expires_at
    ? leaseLeft(card.thread.assignment_expires_at, now)
    : ago(card.thread.updated_at, now);
}

const WAIT_WORDS: Record<string, string> = {
  waiting: "waiting on a person",
  overdue: "overdue on a person",
  returned_to_queue: "returning to the queue",
};

/** The small chips on a card's foot, in the order /ui shows them. */
export function chips(th: Thread, lane: LaneKey): Array<{ kind: string; text: string; title?: string }> {
  const out: Array<{ kind: string; text: string; title?: string }> = [];
  if (th.status) out.push({ kind: `status-${th.status.status}`, text: th.status.status.replace(/_/g, " "), title: th.status.note || `Agent status: ${th.status.status}` });
  if (th.stalled) out.push({ kind: "stalled", text: "stalled", title: "No recent progress from its agent" });
  const wait = th.human_wait && WAIT_WORDS[th.human_wait.state] ? th.human_wait : null;
  if (wait) out.push({ kind: `wait-${wait.state}`, text: WAIT_WORDS[wait.state] });
  if (th.block) out.push({ kind: "blocked", text: `blocked: ${th.block.reason}`, title: th.block.note || `Blocked (${th.block.reason})` });
  if (!th.assignee_id && lane !== "done") out.push({ kind: "unclaimed", text: "unclaimed" });
  return out;
}

/** The card's accessible name: its title, state, and anything urgent. */
export function cardAriaLabel(card: Card): string {
  const th = card.thread;
  const wait = th.human_wait && WAIT_WORDS[th.human_wait.state];
  return `${card.title}: ${card.label}${th.stalled ? ", stalled" : ""}${wait ? `, ${wait}` : ""}`;
}

export type CloseFacts = { verb: "completed by" | "closed by"; actorId: string; at: string | null; handedOffBy: string | null };

/**
 * Who closed a task, from its transitions (#1792). A close that went through
 * review says "completed by"; one that skipped review keeps "closed by". The
 * person who handed it into review is named when that was someone else.
 */
export function closeFacts(th: Thread, rows: Transition[]): CloseFacts | null {
  let closed: Transition | null = null;
  let handedOff: string | null = null;
  for (const row of rows) {
    if (!row || !row.actor_id) continue;
    if (row.to_state === "closed") closed = row;
    if (row.to_state === "in_review") handedOff = row.actor_id;
  }
  if (!closed || !closed.actor_id) return null;
  return {
    verb: th.closed_without_review ? "closed by" : "completed by",
    actorId: closed.actor_id,
    at: closed.at ?? closed.created_at ?? null,
    handedOffBy: handedOff && handedOff !== closed.actor_id ? handedOff : null,
  };
}

/** Which actions the sheet offers on a task, and why one is held back. */
export type TransitionAction = "start_review" | "close" | "archive";
export const TRANSITION_WORDS: Record<TransitionAction, string> = { start_review: "Send to review", close: "Close", archive: "Archive" };

/** The moves the server accepts from this state, close aside (it has its own button). */
export function transitionsFor(th: Thread): TransitionAction[] {
  if (th.state === "archived") return [];
  if (th.state === "closed") return ["archive"];
  return th.state === "in_review" ? ["archive"] : ["start_review", "archive"];
}

/** Parse a typed result: JSON when it parses, else the text as a note. */
export function parseResult(raw: string): { ok: true; value: unknown } | { ok: false; why: string } {
  const t = raw.trim();
  if (!t) return { ok: false, why: "Write the result first." };
  try {
    return { ok: true, value: JSON.parse(t) };
  } catch {
    return { ok: true, value: { summary: t } };
  }
}

export function actionsFor(th: Thread, me: string | null, canClaim: boolean): { claim: string | null | false; close: boolean } {
  const terminal = th.state === "closed" || th.state === "archived";
  let claim: string | null | false = false;
  if (!terminal && !th.assignee_id) claim = canClaim ? null : "Claiming needs a token: an agent claims through the API.";
  return { claim, close: !terminal && (th.state === "in_review" || th.assignee_id === me) };
}

/** Event kinds that move a card. Anything else leaves the board as it is. */
export const BOARD_KINDS = new Set([
  "thread_created",
  "thread_state_changed",
  "thread_assignment_changed",
  "thread_result_set",
  "thread_status_declared",
  "thread_blocked",
  "thread_unblocked",
  "approval_gate_created",
  "approval_gate_resolved",
  "review_submitted",
]);

export function movesBoard(kind: string | undefined): boolean {
  return Boolean(kind && BOARD_KINDS.has(kind));
}

export type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

export class BoardError extends Error {}

async function fail(res: Response, mode: CredentialMode | null): Promise<never> {
  let detail: string | null = null;
  try {
    const body = (await res.json()) as { error?: string; message?: string; detail?: string };
    detail = body.message ?? body.detail ?? body.error ?? null;
  } catch {
    detail = null;
  }
  // A refused move is the one place the server's own sentence is shown, as
  // the classic board keeps it: the agent got the same words.
  if (res.status === 409 && detail) throw new BoardError(`Refused: ${detail}`);
  throw new BoardError(humanError(res.status, detail, mode));
}

/** The reads and writes behind the board. Reads use /ui/api with a session. */
export class BoardApi {
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

  async channels(workspaceId: string): Promise<Channel[]> {
    const rows = await this.get<Array<{ id: string; name: string; kind?: string }>>(`/workspaces/${workspaceId}/channels`);
    return rows.map((r) => ({ id: r.id, name: r.name }));
  }

  async members(workspaceId: string): Promise<Member[]> {
    const rows = await this.get<Array<{ id: string; display_name?: string | null; handle?: string; kind?: string }>>(`/workspaces/${workspaceId}/members`);
    return rows.map((m) => ({ id: m.id, name: m.display_name || m.handle || m.id.slice(0, 8), kind: m.kind }));
  }

  /** Every task in the channel: pages until a short page or a stuck cursor. */
  async threads(channelId: string, pageSize = 500): Promise<Thread[]> {
    const out: Thread[] = [];
    let cursor: string | null = null;
    for (;;) {
      const q = new URLSearchParams({ limit: String(pageSize) });
      if (cursor) q.set("cursor", cursor);
      const batch = await this.get<Thread[]>(`/channels/${channelId}/threads?${q}`);
      out.push(...batch);
      if (batch.length < pageSize) break;
      const next = batch[batch.length - 1].id;
      if (next === cursor) break;
      cursor = next;
    }
    return out;
  }

  /** Pending gates by thread. Best effort: a failed read shows no gates. */
  async gates(workspaceId: string): Promise<Record<string, Gate>> {
    const map: Record<string, Gate> = {};
    try {
      const c = this.creds();
      const res = await this.fetchFn(uiPath(c, `/workspaces/${workspaceId}/approval-gates`), { headers: headers(c), credentials: "include" });
      if (!res.ok) return map;
      const views = (await res.json()) as Array<{ gate?: { thread_id?: string; schema?: unknown } }>;
      for (const v of views) if (v.gate && v.gate.thread_id) map[v.gate.thread_id] = { hasSchema: v.gate.schema != null };
    } catch {
      /* best effort */
    }
    return map;
  }

  transitions(threadId: string): Promise<Transition[]> {
    return this.get<Transition[]>(`/threads/${threadId}/transitions?limit=500`);
  }

  /** Claim with a token. A session has no claim route; the sheet says so. */
  /** Record a task's result. Token only, and it needs the claim's lease id. */
  async setResult(threadId: string, result: unknown, leaseId: string): Promise<void> {
    const c = this.creds();
    const why = bearerRefusal(c);
    if (why) throw new Error(why);
    const res = await this.fetchFn(`${c.base.replace(/\/$/, "")}/threads/${threadId}/result`, writeInit({ method: "PUT", headers: headers(c, true), credentials: "include", body: JSON.stringify({ result, claim_lease_id: leaseId.trim() }) }));
    if (!res.ok) await fail(res, this.mode());
  }

  async claim(threadId: string): Promise<void> {
    const c = this.creds();
    const res = await this.fetchFn(`${c.base.replace(/\/$/, "")}/threads/${threadId}/assignee/claim`, writeInit({ method: "POST", headers: headers(c, true), credentials: "include", body: "{}" }));
    if (!res.ok) await fail(res, this.mode());
  }

  close(threadId: string): Promise<Thread> {
    return this.transition(threadId, "close");
  }

  /** Move a task through the state machine: start_review, close or archive. */
  async transition(threadId: string, action: TransitionAction): Promise<Thread> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/threads/${threadId}`), writeInit({ method: "POST", headers: headers(c, true), credentials: "include", body: JSON.stringify({ action }) }));
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as Thread;
  }
}
