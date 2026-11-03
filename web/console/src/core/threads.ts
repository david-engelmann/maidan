// Threads and DMs: a task's conversation, one-to-one DMs and group DMs. A
// port of the classic board's thread.js and dm.js with the DOM taken out.
// Fetch is passed in, so it is tested.
import { guarded, apiPath, bearerRefusal, headers, failure, ApiError, trimBase, writeInit, type CredentialMode, type Credentials } from "./api.ts";
import type { Fetch, Member, Thread } from "./board.ts";

export type Message = {
  id: string;
  thread_id: string;
  author_id: string;
  body: string;
  metadata?: Record<string, unknown> | null;
  posted_at: string;
  edited_at?: string | null;
  tombstoned_at?: string | null;
};

export type Dm = { id: string; member_low_id: string; member_high_id: string; thread_id: string; updated_at?: string };
export type GroupDm = { id: string; thread_id: string; title?: string | null; member_ids: string[]; updated_at?: string };

/** What a conversation is, as a deep link names it. */
export type ConvRef = { kind: "task"; id: string } | { kind: "dm"; id: string } | { kind: "group"; id: string };

/** A conversation with the thread it reads from and the words it is shown by. */
export type Conversation = { ref: ConvRef; threadId: string; label: string; detail: string; updatedAt: string | null };

const SEG: Record<ConvRef["kind"], string> = { task: "t", dm: "dm", group: "g" };

/** #/threads/t/{id}, #/threads/dm/{id}, #/threads/g/{id}. */
export function convHash(ref: ConvRef): string {
  return `#/threads/${SEG[ref.kind]}/${ref.id}`;
}

export function parseConv(hash: string): ConvRef | null {
  const parts = hash.replace(/^#\/?/, "").split("/");
  if (parts[0] !== "threads" || !parts[2]) return null;
  const id = decodeURIComponent(parts[2]);
  if (parts[1] === "t") return { kind: "task", id };
  if (parts[1] === "dm") return { kind: "dm", id };
  if (parts[1] === "g") return { kind: "group", id };
  return null;
}

export function sameRef(a: ConvRef | null, b: ConvRef | null): boolean {
  return !!a && !!b && a.kind === b.kind && a.id === b.id;
}

/** The person on the other end of a DM. */
export function dmOther(dm: Dm, me: string): string {
  return dm.member_low_id === me ? dm.member_high_id : dm.member_low_id;
}

const settled = new WeakSet<Map<string, Member>>();
/** Marks a member map as the answer of a finished read, even an empty one
 * from a failed read, so names fall back to an id prefix instead of waiting. */
export function settleMembers<T extends Map<string, Member>>(members: T): T {
  settled.add(members);
  return members;
}
/** Whether the member read behind this map is still out. */
export function membersPending(members: Map<string, Member>): boolean {
  return members.size === 0 && !settled.has(members);
}

/** A member's name. Until the member list has loaded this is "", so screens
 * show a placeholder rather than flashing a raw id. */
export function nameOf(members: Map<string, Member>, id: string): string {
  return members.get(id)?.name ?? (membersPending(members) ? "" : id.slice(0, 8));
}

export function dmConversation(dm: Dm, me: string, members: Map<string, Member>): Conversation {
  const other = dmOther(dm, me);
  return { ref: { kind: "dm", id: dm.id }, threadId: dm.thread_id, label: nameOf(members, other), detail: members.get(other)?.kind === "agent" ? "Agent" : "Direct message", updatedAt: dm.updated_at ?? null };
}

export function groupConversation(g: GroupDm, me: string, members: Map<string, Member>): Conversation {
  const others = g.member_ids.filter((id) => id !== me).map((id) => nameOf(members, id));
  return { ref: { kind: "group", id: g.id }, threadId: g.thread_id, label: g.title || "(untitled)", detail: others.every(Boolean) ? `${g.member_ids.length} members · ${others.join(", ")}` : `${g.member_ids.length} members`, updatedAt: g.updated_at ?? null };
}

export function taskConversation(t: Thread): Conversation {
  return { ref: { kind: "task", id: t.id }, threadId: t.id, label: t.title || "(untitled task)", detail: t.state.replace(/_/g, " "), updatedAt: t.updated_at ?? null };
}

/** Most recently active first. */
export function byRecent(a: Conversation, b: Conversation): number {
  return (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");
}

/** Messages a screen shows: removed ones drop out, oldest first. */
export function visibleMessages(rows: Message[]): Message[] {
  return rows.filter((m) => !m.tombstoned_at).sort((a, b) => a.posted_at.localeCompare(b.posted_at) || a.id.localeCompare(b.id));
}

/** Consecutive messages from one author within five minutes share a header. */
/** One header per run: a new author or a new day starts a group. */
export function startsGroup(prev: Message | undefined, m: Message): boolean {
  if (!prev || prev.author_id !== m.author_id) return true;
  return dayKey(prev.posted_at) !== dayKey(m.posted_at);
}

/** The local calendar day a timestamp falls on, as YYYY-MM-DD. */
export function dayKey(ts: string): string {
  const d = new Date(ts);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** "Today", "Yesterday", or a short date for a day separator. */
export function dayLabel(ts: string, now: number): string {
  const key = dayKey(ts);
  if (key === dayKey(new Date(now).toISOString())) return "Today";
  // The calendar day before, not 24 hours before: a DST day is 23 or 25 hours.
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (key === dayKey(yesterday.toISOString())) return "Yesterday";
  const d = new Date(ts);
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric", ...(sameYear ? {} : { year: "numeric" }) });
}

/** Last-opened times per thread, kept on this device. A first visit sets
 *  the baseline, so nothing older than that reads as unread. */
export type SeenState = { baseline: string; opened: Record<string, string> };

export function isUnread(c: Conversation, seen: SeenState): boolean {
  if (!c.updatedAt) return false;
  const mark = seen.opened[c.threadId] ?? seen.baseline;
  // As instants: the server's fractional digits differ from the browser's,
  // so string order is not time order within a second.
  const at = Date.parse(c.updatedAt);
  const seenAt = Date.parse(mark);
  if (Number.isNaN(at) || Number.isNaN(seenAt)) return c.updatedAt > mark;
  return at > seenAt;
}

export function markOpened(seen: SeenState, threadId: string, at: string): SeenState {
  return { ...seen, opened: { ...seen.opened, [threadId]: at } };
}

/** Reactions with one toggled, the way the server will count it. */
export function toggleReaction(cur: Reaction[], emoji: string, on: boolean): Reaction[] {
  const found = cur.find((r) => r.emoji === emoji);
  if (found) {
    if (found.mine === on) return cur;
    return cur.map((r) => (r.emoji === emoji ? { ...r, count: r.count + (on ? 1 : -1), mine: on } : r)).filter((r) => r.count > 0);
  }
  return on ? [...cur, { emoji, count: 1, mine: true }] : cur;
}

export function clockTime(ts: string): string {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export const QUICK_REACTIONS = ["👍", "❤️", "✅", "🎉", "👀"] as const;
export const REACTION_NAMES: Record<string, string> = { "👍": "thumbs up", "❤️": "heart", "✅": "check mark", "🎉": "party popper", "👀": "eyes" };

export type Reaction = { emoji: string; count: number; mine: boolean };

export function aggregateReactions(rows: Array<{ member_id: string; emoji: string }>, me: string | null): Reaction[] {
  const map = new Map<string, Reaction>();
  for (const r of rows) {
    const cur = map.get(r.emoji) ?? { emoji: r.emoji, count: 0, mine: false };
    cur.count += 1;
    if (r.member_id === me) cur.mine = true;
    map.set(r.emoji, cur);
  }
  return [...map.values()];
}

export type SlashResult = { head: string; tone: "ok" | "warn" | "err"; status: string; response: string | null };

/** The result block a slash command leaves in a message's metadata. */
export function slashResult(meta: unknown): SlashResult | null {
  if (!meta || typeof meta !== "object") return null;
  const m = meta as { slash_command?: { name?: string; args?: string }; slash_response?: Record<string, unknown> };
  if (!m.slash_command) return null;
  const sc = m.slash_command;
  const sr = (m.slash_response ?? {}) as { ok?: boolean; retrying?: boolean; error?: string; error_kind?: string; exit_code?: number | null; response?: unknown };
  const head = `/${sc.name ?? ""}${sc.args ? " " + sc.args : ""}`;
  let tone: SlashResult["tone"];
  let status: string;
  if (sr.ok) {
    tone = "ok";
    status = "ok";
  } else if (sr.retrying) {
    tone = "warn";
    status = `retrying${sr.error ? " · " + sr.error : ""}`;
  } else {
    tone = "err";
    const kind = sr.error_kind ? `${sr.error_kind} · ` : "";
    const code = sr.exit_code !== undefined && sr.exit_code !== null ? ` (exit ${sr.exit_code})` : "";
    status = `${kind}${sr.error || "failed"}${code}`;
  }
  const response = sr.response === undefined ? null : typeof sr.response === "string" ? sr.response : JSON.stringify(sr.response, null, 2);
  return { head, tone, status, response };
}

/** Frames that change what an open conversation shows. */
export const THREAD_CONTENT_KINDS = new Set([
  "message_posted",
  "message_edited",
  "message_tombstoned",
  "reaction_added",
  "reaction_removed",
  "message_pinned",
  "message_unpinned",
  "vote_cast",
  "vote_retracted",
]);

/** Frames that change the list of conversations. */
const LIST_KINDS = new Set(["message_posted", "thread_created", "thread_state_changed"]);

export function touchesThread(frame: { kind?: string; thread_id?: unknown }, threadId: string | null): boolean {
  return !!threadId && frame.thread_id === threadId && THREAD_CONTENT_KINDS.has(frame.kind ?? "");
}

export function movesList(kind: string | undefined): boolean {
  return LIST_KINDS.has(kind ?? "");
}

/** Within this many pixels of the end counts as reading the newest message. */
export const AT_BOTTOM_PX = 48;

export function atBottom(el: { scrollTop: number; scrollHeight: number; clientHeight: number }): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= AT_BOTTOM_PX;
}

/**
 * What the list does when its messages change: follow the end only when the
 * reader was already there or just sent one; otherwise hold still and count
 * what arrived below.
 */
export function onNewMessages(wasAtBottom: boolean, sentByMe: boolean, before: string[], after: string[]): { follow: boolean; unseen: number } {
  const known = new Set(before);
  const added = after.filter((id) => !known.has(id)).length;
  if (wasAtBottom || sentByMe || before.length === 0) return { follow: true, unseen: 0 };
  return { follow: false, unseen: added };
}

/** Pixels the on-screen keyboard covers, from the visual viewport. */
export function keyboardInset(innerHeight: number, vv: { height: number; offsetTop: number } | null | undefined): number {
  if (!vv) return 0;
  return Math.max(0, Math.round(innerHeight - vv.height - vv.offsetTop));
}

export const BODY_REQUIRED = "Message body required";
export const CHOOSE_WHO = "Choose who to message";
export const GROUP_MIN = "A group DM needs at least 3 members";
export const NOT_YOURSELF = "You cannot message yourself";

/** Why a group DM cannot open yet, or null when it can. Members include me. */
export function groupRefusal(me: string, picked: string[]): string | null {
  const ids = new Set([me, ...picked]);
  return ids.size < 3 ? GROUP_MIN : null;
}

export type Edit = { edited_at: string; editor_id: string; body_before: string; body_after: string };

/** Pinned messages in the order they appear in the thread; pins whose message is not loaded are left out. */
export function pinnedMessages(rows: Message[], pinned: ReadonlySet<string>): Message[] {
  return rows.filter((m) => pinned.has(m.id) && !m.tombstoned_at);
}

export const EDIT_UNCHANGED = "Nothing changed.";

/** Why an edit cannot go out, or null when it can. */
export function editRefusal(before: string, after: string): string | null {
  if (!after.trim()) return BODY_REQUIRED;
  if (after.trim() === before.trim()) return EDIT_UNCHANGED;
  return null;
}

export type TaskResult = { result: unknown; produced_by?: string | null };
export type Dependencies = { dependencies?: Array<{ depends_on_thread_id: string }>; ready?: boolean };
/** What an evidence record is evidence of; the server's `EvidenceRecordKind`. */
export const EVIDENCE_KINDS = ["test_run", "ci_run", "diff", "review_verdict", "screenshot", "note"] as const;
export type EvidenceKind = (typeof EVIDENCE_KINDS)[number];

export const EVIDENCE_KIND_WORD: Record<EvidenceKind, string> = {
  test_run: "Test run",
  ci_run: "CI run",
  diff: "Diff",
  review_verdict: "Outside review",
  screenshot: "Screenshot",
  note: "Note",
};

/** One record from `GET /threads/{id}/evidence-records`. Records are never edited. */
export type EvidenceRecord = {
  id: string;
  thread_id: string;
  kind: string;
  source: string;
  url?: string | null;
  sha256?: string | null;
  note?: string | null;
  recorded_by: string;
  recorded_at: string;
};

/** A record to add, as typed in the form. Blank fields count as absent. */
export type EvidenceDraft = { kind: EvidenceKind; source: string; url: string; sha256: string; note: string };

export const EVIDENCE_SOURCE_MAX = 200;
export const EVIDENCE_URL_MAX = 2048;
export const EVIDENCE_NOTE_MAX = 4000;

/** Why the server would refuse a draft, or null. Mirrors `NewEvidenceRecord::normalized`. */
export function evidenceRefusal(d: EvidenceDraft): string | null {
  const source = d.source.trim();
  const url = d.url.trim();
  const sha = d.sha256.trim();
  const note = d.note.trim();
  if (!source || [...source].length > EVIDENCE_SOURCE_MAX) return `Say where it came from, 1 to ${EVIDENCE_SOURCE_MAX} characters`;
  if (url) {
    if (new TextEncoder().encode(url).length > EVIDENCE_URL_MAX) return `The URL must be at most ${EVIDENCE_URL_MAX} bytes`;
    if (!safeHref(url)) return "The URL must be an http or https URL";
  }
  if (sha && !/^[0-9a-fA-F]{64}$/.test(sha)) return "The sha256 must be 64 hex characters";
  if ([...note].length > EVIDENCE_NOTE_MAX) return `The note must be at most ${EVIDENCE_NOTE_MAX} characters`;
  if (d.kind === "screenshot" && !url) return "A screenshot needs a URL";
  if (d.kind === "note" && !note) return "A note needs words";
  if (!url && !sha && !note) return "Give a URL, a sha256 or a note";
  return null;
}

/** The request body for a draft: trimmed, blanks left out, the digest lowercase. */
export function evidenceBody(d: EvidenceDraft): Record<string, string> {
  const body: Record<string, string> = { kind: d.kind, source: d.source.trim() };
  if (d.url.trim()) body.url = d.url.trim();
  if (d.sha256.trim()) body.sha256 = d.sha256.trim().toLowerCase();
  if (d.note.trim()) body.note = d.note.trim();
  return body;
}

/** A URL the page may link to: only http and https, so a record cannot carry a `javascript:` link. */
export function safeHref(url: string | null | undefined): string | null {
  if (!url || /\s/.test(url)) return null;
  try {
    const u = new URL(url);
    return (u.protocol === "https:" || u.protocol === "http:") && u.host ? u.href : null;
  } catch {
    return null;
  }
}

/** One record in words, with the recorder named by `name`. */
export function recordLine(r: EvidenceRecord, name: (id: string | undefined) => string): string {
  const word = EVIDENCE_KIND_WORD[r.kind as EvidenceKind] ?? r.kind;
  const parts = [`${word} from ${r.source}`, `recorded by ${name(r.recorded_by)}`];
  if (r.sha256) parts.push(`sha256 ${short(r.sha256)}`);
  return parts.join(", ") + (r.note ? `: ${r.note}` : "");
}

/**
 * The timeline with the thread's records in it. The timeline already merges
 * them as `evidence_recorded`; records it lacks (read from the records list)
 * are added at their time, so the list is the record of truth for records.
 */
export function withRecords(entries: EvidenceEntry[], records: EvidenceRecord[] | null): EvidenceEntry[] {
  if (!records) return entries;
  const seen = new Set(entries.flatMap((e) => (e.kind === "evidence_recorded" && e.record ? [e.record.id] : [])));
  const extra = records.filter((r) => !seen.has(r.id)).map((r): EvidenceEntry => ({ kind: "evidence_recorded", at: r.recorded_at, record: r }));
  if (!extra.length) return entries;
  return [...entries, ...extra].sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
}

export type EvidenceEntry = {
  kind: string;
  at: string;
  record?: EvidenceRecord;
  artifact?: { sha256?: string; linked_by?: string };
  sha256?: string;
  produced_by?: string;
  verdict?: { status?: string; decision?: string; reviewer_id?: string; note?: string | null };
  packet?: { requested_by?: string; thread_version?: number; evidence_root?: string; self_reported_only?: boolean };
};
export type Evidence = { entries?: EvidenceEntry[]; evidence_changed_since_review?: boolean };
export type TaskFacts = { result: TaskResult | null; deps: Dependencies | null; evidence: Evidence | null; records: EvidenceRecord[] | null };

const short = (h: string | undefined) => (h ? `${h.slice(0, 12)}…` : "");

/** One evidence entry in words, with member ids named by `name`. */
export function evidenceLine(e: EvidenceEntry, name: (id: string | undefined) => string): string {
  switch (e.kind) {
    case "artifact_linked":
      return `Artifact ${short(e.artifact?.sha256)} linked by ${name(e.artifact?.linked_by)}`;
    case "evidence_recorded":
      return e.record ? recordLine(e.record, name) : "Evidence recorded";
    case "result_recorded":
      return `Result ${short(e.sha256)} recorded by ${name(e.produced_by)}`.replace("Result  ", "Result ");
    case "land_gate_verdict":
      return `Automated check: ${e.verdict?.status ?? "unknown"}`;
    case "review_requested":
      return `Handed to review by ${name(e.packet?.requested_by)} at version ${e.packet?.thread_version ?? "?"}, evidence root ${short(e.packet?.evidence_root)}${e.packet?.self_reported_only ? " (self-reported only)" : ""}`;
    case "review_verdict":
      return `Review: ${String(e.verdict?.decision ?? "").replace(/_/g, " ")} by ${name(e.verdict?.reviewer_id)}${e.verdict?.note ? `: ${e.verdict.note}` : ""}`;
    default:
      return e.kind;
  }
}

export class ThreadsError extends ApiError {}

const fail = (res: Response, mode: CredentialMode | null): Promise<never> =>
  failure(res, mode, ThreadsError, (status, detail) => ((status === 400 || status === 409 || status === 422) && detail ? detail : null));

/** The files on a paste, if any. A paste of text alone returns none, so it stays text. */
export function pastedFiles(data: Pick<DataTransfer, "items"> | null | undefined): File[] {
  if (!data) return [];
  const out: File[] = [];
  for (const item of Array.from(data.items)) {
    if (item.kind !== "file") continue;
    const f = item.getAsFile();
    if (f) out.push(f);
  }
  return out;
}

/** The name a pasted file is stored under: its last path segment, never the path. */
export function attachName(name: string | undefined, type: string): string {
  const base = (name ?? "").split(/[\\/]/).pop()?.trim() ?? "";
  if (base && base !== "." && base !== "..") return base;
  const ext = type.startsWith("image/") ? `.${type.slice(6).split(/[+;]/)[0]}` : "";
  return `pasted${ext}`;
}

/** An image is a screenshot; anything else is an attachment. */
export function attachKind(type: string): "screenshot" | "attachment" {
  return type.startsWith("image/") ? "screenshot" : "attachment";
}

export class ThreadsApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = guarded(fetchFn, ThreadsError);
    this.creds = creds;
    this.mode = mode;
  }

  private async get<T>(path: string): Promise<T> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, path), { headers: headers(c), credentials: "include" });
    if (!res.ok) return fail(res, this.mode());
    return (await res.json()) as T;
  }

  /** A read where a missing record (404) or any failure is just "nothing to show". */
  private async soft<T>(path: string): Promise<T | null> {
    try {
      return await this.get<T>(path);
    } catch {
      return null;
    }
  }

  /** A task's result for a reviewer: none recorded (a 404) is not the same as
   * a read that failed, which says why. */
  async resultRead(threadId: string): Promise<{ result: TaskResult | null; error: string | null }> {
    try {
      return { result: await this.get<TaskResult>(`/threads/${threadId}/result`), error: null };
    } catch (e) {
      if ((e as { status?: number }).status === 404) return { result: null, error: null };
      return { result: null, error: e instanceof Error ? e.message : "Could not load the result" };
    }
  }

  /** Result, dependencies and evidence for a task; each part is optional. */
  async facts(threadId: string): Promise<TaskFacts> {
    const [result, deps, evidence, records] = await Promise.all([
      this.soft<TaskResult>(`/threads/${threadId}/result`),
      this.soft<Dependencies>(`/threads/${threadId}/dependencies`),
      this.soft<Evidence>(`/threads/${threadId}/evidence`),
      this.records(threadId),
    ]);
    return { result, deps, evidence, records };
  }

  /**
   * The thread's evidence records, oldest first, or null when they cannot be
   * read. The records list is a bearer route; a signed-in session reads the
   * same records from the timeline instead.
   */
  async records(threadId: string): Promise<EvidenceRecord[] | null> {
    if (bearerRefusal(this.creds())) return null;
    return this.soft<EvidenceRecord[]>(`/threads/${threadId}/evidence-records`);
  }

  /** Adds a record. Bearer only, like the route; refused drafts never go out. */
  async addEvidence(threadId: string, draft: EvidenceDraft): Promise<EvidenceRecord | null> {
    const why = bearerRefusal(this.creds()) ?? evidenceRefusal(draft);
    if (why) throw new ThreadsError(why);
    return this.send<EvidenceRecord>("POST", `/threads/${threadId}/evidence-records`, evidenceBody(draft));
  }

  private async send<T>(method: string, path: string, body: unknown): Promise<T | null> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, path), writeInit({ method, headers: headers(c, true), credentials: "include", body: JSON.stringify(body) }));
    if (!res.ok) return fail(res, this.mode());
    const text = await res.text();
    return text ? (JSON.parse(text) as T) : null;
  }

  /** Stores a pasted file as an artifact and links it to the thread. Returns the stored name. */
  async attach(threadId: string, file: Blob & { name?: string }): Promise<{ sha256: string; filename: string }> {
    const c = this.creds();
    const filename = attachName(file.name, file.type);
    const params = new URLSearchParams({ kind: attachKind(file.type), filename });
    if (file.type) params.set("mime_type", file.type);
    const res = await this.fetchFn(apiPath(c, `/artifacts?${params}`), writeInit({ method: "POST", headers: headers(c), credentials: "include", body: file }));
    if (!res.ok) return fail(res, this.mode());
    const out = (await res.json()) as { sha256: string; filename?: string | null };
    await this.send("PUT", `/threads/${threadId}/artifacts/${encodeURIComponent(out.sha256)}`, {});
    return { sha256: out.sha256, filename: out.filename || filename };
  }

  thread(id: string): Promise<Thread> {
    return this.get<Thread>(`/threads/${id}`);
  }

  messages(threadId: string): Promise<Message[]> {
    return this.get<Message[]>(`/threads/${threadId}/messages?limit=50`);
  }

  /** Votes on a message, tallied by kind (GET /messages/{id}/votes). */
  async votes(messageId: string, me: string | null): Promise<VoteTally[]> {
    return tallyVotes(await this.get<VoteRow[]>(`/messages/${messageId}/votes`), me);
  }

  async reactions(messageId: string, me: string | null): Promise<Reaction[]> {
    return aggregateReactions(await this.get<Array<{ member_id: string; emoji: string }>>(`/messages/${messageId}/reactions`), me);
  }

  async react(messageId: string, emoji: string, on: boolean): Promise<void> {
    await this.send(on ? "POST" : "DELETE", `/messages/${messageId}/reactions`, { emoji });
  }

  async pins(threadId: string): Promise<string[]> {
    const rows = await this.get<Array<{ message_id?: string }>>(`/threads/${threadId}/pins`);
    return rows.map((r) => String(r.message_id ?? "")).filter(Boolean);
  }

  async pin(threadId: string, messageId: string, on: boolean): Promise<void> {
    await this.send(on ? "POST" : "DELETE", `/threads/${threadId}/pins`, { message_id: messageId });
  }

  /** Delete (tombstone) a message, or purge its body for good (admin). Token only. */
  async remove(id: string, purge = false): Promise<void> {
    const c = this.creds();
    const why = bearerRefusal(c);
    if (why) throw new ThreadsError(why);
    const res = await this.fetchFn(`${trimBase(c.base)}/messages/${id}${purge ? "/purge" : ""}`, writeInit({ method: "DELETE", headers: headers(c, false), credentials: "include" }));
    if (!res.ok) await fail(res, this.mode());
  }

  /** Edits your own message; the server keeps the old body in the history. */
  async edit(m: Pick<Message, "id" | "body">, body: string): Promise<Message | null> {
    const why = editRefusal(m.body, body);
    if (why) throw new ThreadsError(why);
    return this.send<Message>("PATCH", `/messages/${m.id}`, { body: body.trim() });
  }

  /** Newest first, as the server sends them. */
  edits(messageId: string): Promise<Edit[]> {
    return this.get<Edit[]>(`/messages/${messageId}/edits?limit=50`);
  }

  dms(workspaceId: string, me: string): Promise<Dm[]> {
    return this.get<Dm[]>(`/workspaces/${workspaceId}/dm?member_id=${encodeURIComponent(me)}`);
  }

  groupDms(workspaceId: string, me: string): Promise<GroupDm[]> {
    return this.get<GroupDm[]>(`/workspaces/${workspaceId}/group-dms?member_id=${encodeURIComponent(me)}`);
  }

  /** Posts to the route the conversation's kind owns. An empty body is refused here. */
  async post(ref: ConvRef, body: string): Promise<Message | null> {
    const text = body.trim();
    if (!text) throw new ThreadsError(BODY_REQUIRED);
    const path = ref.kind === "task" ? `/threads/${ref.id}/messages` : ref.kind === "dm" ? `/dm/${ref.id}/messages` : `/group-dms/${ref.id}/messages`;
    return this.send<Message>("POST", path, { body: text });
  }

  async openDm(workspaceId: string, me: string, other: string | null): Promise<Dm> {
    if (!other) throw new ThreadsError(CHOOSE_WHO);
    if (other === me) throw new ThreadsError(NOT_YOURSELF);
    const dm = await this.send<Dm>("POST", `/workspaces/${workspaceId}/dm`, { other_member_id: other });
    if (!dm) throw new ThreadsError("The server did not return the conversation");
    return dm;
  }

  async openGroupDm(workspaceId: string, me: string, picked: string[], title: string): Promise<GroupDm> {
    const refusal = groupRefusal(me, picked);
    if (refusal) throw new ThreadsError(refusal);
    const ids = [...new Set([me, ...picked])];
    const g = await this.send<GroupDm>("POST", `/workspaces/${workspaceId}/group-dms`, { member_ids: ids, title: title.trim() || null });
    if (!g) throw new ThreadsError("The server did not return the conversation");
    return g;
  }
}

export type VoteRow = { message_id?: string; member_id: string; kind: string; confidence?: number | null; created_at?: string };
export type VoteTally = { kind: string; count: number; voters: string[]; mine: boolean };

/** What each vote kind is called on a message. */
export const VOTE_WORDS: Record<string, string> = { approve: "approve", request_changes: "changes requested", ack: "ack" };
const VOTE_ORDER = ["approve", "request_changes", "ack"];

/** Votes grouped by kind, approve first, voters in the order they voted. */
export function tallyVotes(rows: VoteRow[], me: string | null): VoteTally[] {
  const by = new Map<string, VoteTally>();
  const sorted = [...rows].sort((a, b) => (Date.parse(a.created_at ?? "") || 0) - (Date.parse(b.created_at ?? "") || 0));
  for (const r of sorted) {
    const t = by.get(r.kind) ?? { kind: r.kind, count: 0, voters: [], mine: false };
    t.count += 1;
    t.voters.push(r.member_id);
    if (me && r.member_id === me) t.mine = true;
    by.set(r.kind, t);
  }
  const rank = (k: string) => (VOTE_ORDER.indexOf(k) + 1 || 99);
  return [...by.values()].sort((a, b) => rank(a.kind) - rank(b.kind) || a.kind.localeCompare(b.kind));
}
