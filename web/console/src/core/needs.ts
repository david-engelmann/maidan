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

/** GET /threads/{id}/review-packet: what a task was handed to review with. */
export type AttestationTier = "verified" | "attached" | "self_reported";
export type Attestation = { kind: "result" | "artifact" | "land_gate"; sha256?: string | null; tier: AttestationTier | string; attested_by: string };
export type ReviewPacket = {
  thread_version?: number;
  requested_by?: string;
  evidence_root: string;
  self_reported_only: boolean;
  manifest: { result?: { sha256: string; produced_by: string } | null; artifacts?: string[]; attestations?: Attestation[] };
};
export type ArtifactMeta = { filename?: string | null; kind?: string | null; size_bytes?: number | null; uploaded_by?: string | null };
export type ArtifactLink = { sha256: string; linked_by?: string | null; linked_at?: string | null };

/** A packet read: one was handed over, none was, or the read failed and says why. */
export type PacketRead = { state: "ready"; packet: ReviewPacket } | { state: "none" } | { state: "error"; why: string };

/** One line of evidence as a row shows it. */
export type EvidenceItem = {
  kind: "result" | "artifact" | "land_gate";
  label: string;
  sha: string | null;
  by: string | null;
  tier: { word: string; title: string; wire: string } | null;
  unlinked: boolean;
};

const TIER_TEXT: Record<string, [string, string]> = {
  verified: ["verified", "A land-gate pass the close gate accepts"],
  attached: ["attached", "Linked by a member who never worked the task"],
  self_reported: ["self-reported", "The work's own account: from a member who worked the task"],
};

function tierOf(a: Attestation | undefined): EvidenceItem["tier"] {
  if (!a || !a.tier) return null;
  const [word, title] = TIER_TEXT[a.tier] ?? [String(a.tier), ""];
  return { word, title, wire: String(a.tier) };
}

export function formatBytes(n: number | null | undefined): string {
  if (typeof n !== "number" || !Number.isFinite(n) || n < 0) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * The evidence a review row shows before Approve: the result, each linked
 * artifact and each land-gate pass, with the tier the server gave it at the
 * hand-off. An artifact the packet pinned that the task no longer links is
 * flagged, since approving that packet is refused. `links` null means the
 * read failed, so nothing is flagged.
 */
export function evidenceItems(packet: ReviewPacket, metas: Map<string, ArtifactMeta | null>, links: Map<string, ArtifactLink> | null): EvidenceItem[] {
  const m = packet.manifest ?? {};
  const att = m.attestations ?? [];
  const of = (k: Attestation["kind"]) => att.filter((a) => a.kind === k);
  const out: EvidenceItem[] = [];
  if (m.result) out.push({ kind: "result", label: "result", sha: m.result.sha256, by: m.result.produced_by, tier: tierOf(of("result")[0]), unlinked: false });
  const arts = of("artifact");
  (m.artifacts ?? []).forEach((sha, i) => {
    const meta = metas.get(sha) ?? null;
    const size = formatBytes(meta?.size_bytes);
    const name = meta ? `${meta.filename || "unnamed"}${size ? ` · ${size}` : ""}` : "artifact (details unavailable)";
    const link = links?.get(sha);
    out.push({
      kind: "artifact",
      label: name,
      sha,
      by: link?.linked_by ?? meta?.uploaded_by ?? null,
      tier: tierOf(arts.find((a) => a.sha256 === sha) ?? arts[i]),
      unlinked: links !== null && !link,
    });
  });
  for (const g of of("land_gate")) out.push({ kind: "land_gate", label: "land-gate pass", sha: g.sha256 ?? null, by: g.attested_by, tier: tierOf(g), unlinked: false });
  return out;
}

export function shortHash(h: string | null | undefined): string {
  return h ? `${h.slice(0, 12)}…` : "";
}

export type ReviewStatus = { approvals: number; required_count: number; approvals_met: boolean };

export type GroupId = "decision" | "action" | "question";
export const GROUP_LABEL: Record<GroupId, string> = {
  decision: "Needs your decision",
  action: "Needs your action",
  question: "An agent asked",
};

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

  /** The packet a review approves: its root is what Approve signs. */
  async reviewPacket(threadId: string): Promise<PacketRead> {
    let res: Response;
    try {
      res = await this.fetchFn(apiPath(this.creds(), `/threads/${threadId}/review-packet`), { headers: headers(this.creds()), credentials: "include" });
    } catch {
      return { state: "error", why: "Could not load the evidence: the server did not answer" };
    }
    if (res.status === 404) return { state: "none" };
    if (!res.ok) return { state: "error", why: `Could not load the evidence (${res.status})` };
    try {
      return { state: "ready", packet: (await res.json()) as ReviewPacket };
    } catch {
      return { state: "error", why: "Could not load the evidence: the server sent something unreadable" };
    }
  }

  /** Who linked each artifact to the task, by hash; null when the read failed. */
  async threadLinks(threadId: string): Promise<Map<string, ArtifactLink> | null> {
    try {
      const rows = await this.read<ArtifactLink[]>(apiPath(this.creds(), `/threads/${threadId}/artifacts`));
      return Array.isArray(rows) ? new Map(rows.filter((r) => r && r.sha256).map((r) => [r.sha256, r])) : null;
    } catch {
      return null;
    }
  }

  async artifactMeta(sha: string): Promise<ArtifactMeta | null> {
    try {
      return await this.read<ArtifactMeta>(uiPath(this.creds(), `/artifacts/${encodeURIComponent(sha)}/meta`));
    } catch {
      return null;
    }
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

  async unblock(threadId: string): Promise<void> {
    await this.write(apiPath(this.creds(), `/threads/${threadId}/block`), "DELETE");
  }

  async close(threadId: string): Promise<void> {
    await this.write(apiPath(this.creds(), `/threads/${threadId}`), "POST", { action: "close" });
  }
}
