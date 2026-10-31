// Threads and DMs: a task's conversation, one-to-one DMs and group DMs. A
// port of the classic board's thread.js and dm.js with the DOM taken out.
// Fetch is passed in, so it is tested.
import { apiPath, headers, humanError, writeInit, type CredentialMode, type Credentials } from "./api.ts";
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

export function nameOf(members: Map<string, Member>, id: string): string {
  return members.get(id)?.name ?? id.slice(0, 8);
}

export function dmConversation(dm: Dm, me: string, members: Map<string, Member>): Conversation {
  const other = dmOther(dm, me);
  return { ref: { kind: "dm", id: dm.id }, threadId: dm.thread_id, label: nameOf(members, other), detail: members.get(other)?.kind === "agent" ? "Agent" : "Direct message", updatedAt: dm.updated_at ?? null };
}

export function groupConversation(g: GroupDm, me: string, members: Map<string, Member>): Conversation {
  const others = g.member_ids.filter((id) => id !== me).map((id) => nameOf(members, id));
  return { ref: { kind: "group", id: g.id }, threadId: g.thread_id, label: g.title || "(untitled)", detail: `${g.member_ids.length} members · ${others.join(", ")}`, updatedAt: g.updated_at ?? null };
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
export function startsGroup(prev: Message | undefined, m: Message): boolean {
  if (!prev || prev.author_id !== m.author_id) return true;
  return Date.parse(m.posted_at) - Date.parse(prev.posted_at) > 5 * 60 * 1000;
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

export class ThreadsError extends Error {}

async function fail(res: Response, mode: CredentialMode | null): Promise<never> {
  let detail: string | null = null;
  try {
    const body = (await res.json()) as { error?: string; message?: string; detail?: string };
    detail = body.message ?? body.detail ?? body.error ?? null;
  } catch {
    detail = null;
  }
  if ((res.status === 400 || res.status === 409 || res.status === 422) && detail) throw new ThreadsError(detail);
  throw new ThreadsError(humanError(res.status, detail, mode));
}

export class ThreadsApi {
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

  private async send<T>(method: string, path: string, body: unknown): Promise<T | null> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, path), writeInit({ method, headers: headers(c, true), credentials: "include", body: JSON.stringify(body) }));
    if (!res.ok) return fail(res, this.mode());
    const text = await res.text();
    return text ? (JSON.parse(text) as T) : null;
  }

  thread(id: string): Promise<Thread> {
    return this.get<Thread>(`/threads/${id}`);
  }

  messages(threadId: string): Promise<Message[]> {
    return this.get<Message[]>(`/threads/${threadId}/messages?limit=50`);
  }

  async reactions(messageId: string, me: string | null): Promise<Reaction[]> {
    return aggregateReactions(await this.get<Array<{ member_id: string; emoji: string }>>(`/messages/${messageId}/reactions`), me);
  }

  async react(messageId: string, emoji: string, on: boolean): Promise<void> {
    await this.send(on ? "POST" : "DELETE", `/messages/${messageId}/reactions`, { emoji });
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
