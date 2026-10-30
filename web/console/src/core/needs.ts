// Needs you: the reviews, approval gates, blocks and questions waiting on the
// signed-in member. A port of the classic board's needs.js and the gate half
// of tools.js, with the DOM taken out. Fetch is passed in, so it is tested.
import { apiPath, headers, humanError, uiPath, writeInit, type CredentialMode, type Credentials } from "./api.ts";
import { BOARD_KINDS, type Fetch } from "./board.ts";

export type NeedKind = "review_request" | "unassigned_review" | "open_gate" | "blocked" | "question";
const KINDS: ReadonlySet<string> = new Set<NeedKind>(["review_request", "unassigned_review", "open_gate", "blocked", "question"]);

/** One row of GET /members/{id}/waiting. */
export type WaitingItem = {
  kind: string;
  thread_id?: string | null;
  gate_id?: string | null;
  message_id?: string | null;
  summary: string;
  detail?: string | null;
  since: string;
  age_secs: number;
  overdue?: boolean;
};

export type Need = WaitingItem & { kind: NeedKind };

export type ModelRequest = { client_name?: string | null; client_version?: string | null; client_source?: string | null };

/** A pending gate and the signed token its answer must echo. */
export type GateView = {
  gate: { id: string; thread_id?: string | null; prompt?: string; requested_by?: string | null; schema?: unknown };
  request_state: string;
  model_request?: ModelRequest | null;
};

export type ReviewStatus = { approvals: number; required_count: number; approvals_met: boolean };

export type GroupId = "decision" | "action" | "question";
export const GROUP_LABEL: Record<GroupId, string> = {
  decision: "Needs your decision",
  action: "Needs your action",
  question: "An agent asked",
};

/** A `#confirm-approval=<gate>.<token>` link from a confirmation email or chat, or null. */
export type ConfirmLink = { gateId: string; token: string };
export function parseConfirmLink(hash: string): ConfirmLink | null {
  const m = /^#\/?confirm-approval=([^.&]+)\.([^&]+)$/.exec(hash);
  if (!m) return null;
  try {
    return { gateId: decodeURIComponent(m[1]), token: decodeURIComponent(m[2]) };
  } catch {
    return null;
  }
}

export function isNeed(it: WaitingItem): it is Need {
  return KINDS.has(it.kind);
}

export function groupOf(kind: NeedKind): GroupId {
  if (kind === "blocked") return "action";
  if (kind === "question") return "question";
  return "decision";
}

const RANK: Record<GroupId, number> = { decision: 0, action: 1, question: 2 };

/** A row's identity across reloads, so focus and in-flight state survive. */
export function needKey(it: WaitingItem): string {
  return `${it.kind}:${it.thread_id ?? ""}:${it.gate_id ?? ""}`;
}

/** Decisions first, then actions, then questions; a named review leads, newest first. */
export function sortNeeds(items: WaitingItem[]): Need[] {
  return items.filter(isNeed).sort((a, b) => {
    const g = RANK[groupOf(a.kind)] - RANK[groupOf(b.kind)];
    if (g) return g;
    const r = Number(b.kind === "review_request") - Number(a.kind === "review_request");
    if (r) return r;
    return Date.parse(b.since) - Date.parse(a.since);
  });
}

export type NeedGroup = { id: GroupId; label: string; items: Need[] };

/** Groups in order. Headings only help when there is more than one group. */
export function groupNeeds(sorted: Need[]): { groups: NeedGroup[]; headings: boolean } {
  const groups: NeedGroup[] = [];
  for (const it of sorted) {
    const id = groupOf(it.kind);
    let g = groups.find((x) => x.id === id);
    if (!g) groups.push((g = { id, label: GROUP_LABEL[id], items: [] }));
    g.items.push(it);
  }
  return { groups, headings: groups.length > 1 };
}

export function kindLabel(kind: NeedKind): string {
  if (kind === "open_gate") return "Approval";
  if (kind === "blocked") return "Blocked";
  if (kind === "question") return "Question";
  return "Review";
}

/** "title — blocked (reason): note" → the title, and the part that says why. */
export function splitBlocked(summary: string): { title: string; why: string } {
  const s = String(summary || "");
  const at = s.indexOf(" — blocked");
  if (at < 0) return { title: s, why: s };
  return { title: s.slice(0, at), why: s.slice(at + 3) };
}

/** "title: question", with `detail` the question alone. The title may hold ": ". */
export function splitQuestion(it: WaitingItem): { title: string; question: string } {
  const q = it.detail ?? "";
  const tail = `: ${q}`;
  if (q && it.summary.endsWith(tail)) return { title: it.summary.slice(0, -tail.length), question: q };
  return { title: it.summary, question: q || it.summary };
}

export function waitingFor(ageSecs: number): string {
  const s = Math.max(0, Math.floor(ageSecs));
  if (s < 60) return "waiting under a minute";
  if (s < 3600) return `waiting ${Math.floor(s / 60)}m`;
  if (s < 86400) return `waiting ${Math.floor(s / 3600)}h`;
  return `waiting ${Math.floor(s / 86400)}d`;
}

export function clientLabel(r: ModelRequest | null | undefined): string {
  if (!r || !r.client_name) return "an unidentified MCP client";
  if (r.client_source === "credential") return r.client_name;
  return `${r.client_name}${r.client_version ? " " + r.client_version : ""} (self-reported)`;
}

export function modelRequestLine(r: ModelRequest | null | undefined): string | null {
  return r ? `a model asked via ${clientLabel(r)}, waiting for you to confirm` : null;
}

export function approvedLine(rs: ReviewStatus | null): string {
  return rs && rs.required_count ? `Approved ✓ ${rs.approvals}/${rs.required_count}` : "Approved ✓";
}

/** Live frames that can change what waits on you. */
export function movesNeeds(kind: string | undefined): boolean {
  return Boolean(kind && (BOARD_KINDS.has(kind) || kind.startsWith("message_")));
}

/** What a row is doing. One thing at a time, so a second tap cannot race the first. */
export type Ask = "approve" | "approve_note" | "request_changes" | "decline" | "accept" | "unblock" | "close";
export type RowState =
  | { phase: "idle" }
  | { phase: "confirm"; ask: Ask }
  | { phase: "note"; ask: "approve_note" | "request_changes" | "decline" }
  | { phase: "sending"; ask: Ask }
  | { phase: "done"; text: string; tone: "ok" | "no"; closable: boolean }
  | { phase: "error"; message: string };

/** Which asks need words, and whether the words are required. */
export const NOTE_ASK: Record<"approve_note" | "request_changes" | "decline", { prompt: string; required: boolean; send: string }> = {
  request_changes: { prompt: "What should change? The agent reads this.", required: true, send: "Send back" },
  approve_note: { prompt: "Anything the agent should know? Optional.", required: false, send: "Approve" },
  decline: { prompt: "Why decline? Optional. The agent reads this.", required: false, send: "Decline" },
};

export class NeedsError extends Error {}

async function fail(res: Response, mode: CredentialMode | null): Promise<never> {
  let detail: string | null = null;
  try {
    const b = (await res.json()) as { error?: string; message?: string; detail?: string };
    detail = b.message ?? b.detail ?? b.error ?? null;
  } catch {
    detail = null;
  }
  if (res.status === 409) throw new NeedsError(detail ? `Refused: ${detail}` : "Someone already answered this");
  throw new NeedsError(humanError(res.status, detail, mode));
}

/** The reads and writes behind Needs you. */
export class NeedsApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = fetchFn;
    this.creds = creds;
    this.mode = mode;
  }

  private async read<T>(url: string, missing?: T): Promise<T> {
    const c = this.creds();
    const res = await this.fetchFn(url, { headers: headers(c), credentials: "include" });
    if (res.status === 404 && missing !== undefined) return missing;
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as T;
  }

  private async write(url: string, method: string, body?: unknown): Promise<Response> {
    const c = this.creds();
    const init: RequestInit = { method, headers: headers(c, body !== undefined), credentials: "include" };
    if (body !== undefined) init.body = JSON.stringify(body);
    const res = await this.fetchFn(url, writeInit(init));
    if (!res.ok) return fail(res, this.mode());
    return res;
  }

  async waiting(memberId: string): Promise<Need[]> {
    const body = await this.read<{ items?: WaitingItem[] }>(uiPath(this.creds(), `/members/${memberId}/waiting`));
    return sortNeeds(body.items ?? []);
  }

  /** Pending gates by gate id. Best effort: a failed read leaves gate rows without buttons. */
  async gates(workspaceId: string): Promise<Map<string, GateView>> {
    try {
      const rows = await this.read<GateView[]>(uiPath(this.creds(), `/workspaces/${workspaceId}/approval-gates`));
      return new Map(rows.map((v) => [v.gate.id, v]));
    } catch {
      return new Map();
    }
  }

  /** The evidence an approval signs, or null when nothing was handed over. */
  async reviewRoot(threadId: string): Promise<string | null> {
    const p = await this.read<{ evidence_root?: string | null } | null>(apiPath(this.creds(), `/threads/${threadId}/review-packet`), null);
    return p?.evidence_root ?? null;
  }

  async reviewStatus(threadId: string): Promise<ReviewStatus | null> {
    try {
      return await this.read<ReviewStatus>(apiPath(this.creds(), `/threads/${threadId}/review-status`));
    } catch {
      return null;
    }
  }

  async ownerOf(threadId: string): Promise<string | null> {
    try {
      return (await this.read<{ owner_id?: string | null }>(uiPath(this.creds(), `/threads/${threadId}`))).owner_id ?? null;
    } catch {
      return null;
    }
  }

  async review(threadId: string, decision: "approve" | "request_changes", note: string, evidenceRoot: string | null): Promise<void> {
    const body: Record<string, string> = { decision };
    if (note.trim()) body.note = note.trim();
    if (decision === "approve" && evidenceRoot) body.evidence_root = evidenceRoot;
    await this.write(apiPath(this.creds(), `/threads/${threadId}/reviews`), "POST", body);
  }

  /** Accept or decline. A reason rides in `content.note`, the gate's response detail. */
  async answerGate(view: GateView, action: "accept" | "decline", note = ""): Promise<void> {
    const body: Record<string, unknown> = { request_state: view.request_state, action };
    if (note.trim()) body.content = { note: note.trim() };
    await this.write(apiPath(this.creds(), `/approval-gates/${view.gate.id}/answer`), "POST", body);
  }

  /**
   * Confirm a gate from a signed link. The server takes this on the browser
   * session only, so no bearer header rides along, and the link token is
   * never stored or shown.
   */
  async confirmLink(link: ConfirmLink): Promise<void> {
    const c = this.creds();
    const init = writeInit({
      method: "POST",
      headers: { "Content-Type": "application/json" },
      credentials: "include",
      body: JSON.stringify({ gate_id: link.gateId, token: link.token }),
    });
    const res = await this.fetchFn(`${c.base}/auth/approval-confirmations/confirm`, init);
    if (!res.ok) await fail(res, this.mode());
  }

  async unblock(threadId: string): Promise<void> {
    await this.write(apiPath(this.creds(), `/threads/${threadId}/block`), "DELETE");
  }

  async close(threadId: string): Promise<void> {
    await this.write(apiPath(this.creds(), `/threads/${threadId}`), "POST", { action: "close" });
  }
}
