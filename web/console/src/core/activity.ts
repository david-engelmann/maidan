// The Activity screen: the workspace event log read once, then kept live
// from the Shell's one connection (liveBus). Rows from the REST read and
// from live frames share one shape and dedupe by log id.

export type ActivityRow = {
  id: number;
  kind: string;
  threadId: string | null;
  channelId: string | null;
  actorId: string | null;
  at: string | null;
  /** Task title carried on the event, when the payload has one. */
  title: string | null;
  /** Channel name carried on the event, when the payload has one. */
  channelName: string | null;
  /** Actor display name carried on the event. */
  actorName: string | null;
  /** Actor handle carried on the event, without a leading @. */
  actorHandle: string | null;
  /** Destination state, when the event is a transition. */
  toState: string | null;
};

/** Names the screen has resolved, so a row can say who and what instead of an id. */
export type ActivityDirectory = {
  threads: Map<string, string>;
  channels: Map<string, string>;
  actors: Map<string, string>;
};

export const EMPTY_DIRECTORY: ActivityDirectory = { threads: new Map(), channels: new Map(), actors: new Map() };

/** The most rows the screen keeps; older ones fall off. */
export const ACTIVITY_CAP = 300;

export const ACTIVITY_GROUPS = [
  { id: "all", title: "All" },
  { id: "tasks", title: "Tasks" },
  { id: "messages", title: "Messages" },
  { id: "votes", title: "Votes" },
  { id: "members", title: "Members" },
] as const;
export type ActivityGroup = (typeof ACTIVITY_GROUPS)[number]["id"];

const str = (v: unknown): string | null => (typeof v === "string" && v ? v : null);

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

function actorOf(o: Record<string, unknown>): string | null {
  const p = (o.payload && typeof o.payload === "object" ? (o.payload as Record<string, unknown>) : {}) as Record<string, unknown>;
  // A freeze names the frozen member in member_id; who did it is frozen_by.
  for (const k of ["actor_id", "frozen_by", "unfrozen_by", "member_id", "author_id", "by"]) {
    const v = str(o[k]) ?? str(p[k]);
    if (v) return v;
  }
  for (const blob of [obj(p.member), obj(p.actor), obj(p.author), obj(o.member), obj(o.actor)]) {
    const id = blob ? str(blob.id) : null;
    if (id) return id;
  }
  return null;
}

function nestedStr(blobs: Array<Record<string, unknown> | null>, key: string): string | null {
  for (const b of blobs) {
    const v = b ? str(b[key]) : null;
    if (v) return v;
  }
  return null;
}

/** A row from GET /workspaces/{id}/events or a live event frame; null if it has no id. */
export function toActivity(o: Record<string, unknown>): ActivityRow | null {
  const id = typeof o.id === "number" ? o.id : typeof o.log_id === "number" ? o.log_id : null;
  if (id === null) return null;
  const p = (o.payload && typeof o.payload === "object" ? (o.payload as Record<string, unknown>) : {}) as Record<string, unknown>;
  const thread = obj(p.thread) ?? obj(o.thread);
  const channel = obj(p.channel) ?? obj(o.channel);
  const people = [obj(p.member), obj(p.actor), obj(p.author), obj(o.member), obj(o.actor)];
  return {
    id,
    kind: str(o.kind) ?? "event",
    threadId: str(o.thread_id) ?? str(p.thread_id) ?? (thread ? str(thread.id) : null),
    channelId: str(o.channel_id) ?? str(p.channel_id) ?? (channel ? str(channel.id) : null),
    actorId: actorOf(o),
    at: str(o.occurred_at) ?? str(p.occurred_at),
    title: str(p.title) ?? (thread ? str(thread.title) : null) ?? str(o.title),
    channelName: str(p.channel_name) ?? (channel ? str(channel.name) : null) ?? str(o.channel_name),
    actorName: nestedStr(people, "display_name") ?? str(p.display_name) ?? str(o.display_name),
    actorHandle: (nestedStr(people, "handle") ?? str(p.handle) ?? str(o.handle))?.replace(/^@/, "") ?? null,
    toState: str(p.to_state) ?? str(o.to_state) ?? str(p.state) ?? str(o.state),
  };
}

/** Newest first, one row per id, at most `cap`. */
export function mergeActivity(rows: ActivityRow[], incoming: ActivityRow[], cap = ACTIVITY_CAP): ActivityRow[] {
  const by = new Map<number, ActivityRow>();
  for (const r of rows) by.set(r.id, r);
  for (const r of incoming) by.set(r.id, r);
  return [...by.values()].sort((a, b) => b.id - a.id).slice(0, cap);
}

/** Task and message events whose kind carries no thread_ or message_ prefix. */
const TASK_KINDS = new Set(["claim_expired", "claim_unacknowledged", "claim_stalled", "claim_failed", "approval_requested", "blocked_resolved", "review_submitted"]);
const MESSAGE_KINDS = new Set(["mention_recorded"]);

export function groupOf(kind: string): Exclude<ActivityGroup, "all"> | null {
  if (kind.startsWith("vote_")) return "votes";
  if (kind.startsWith("message_") || kind.startsWith("reaction_") || MESSAGE_KINDS.has(kind)) return "messages";
  if (kind.startsWith("thread_") || kind.startsWith("task_") || TASK_KINDS.has(kind)) return "tasks";
  if (kind.startsWith("member_") || kind.startsWith("agent_") || kind.startsWith("token_")) return "members";
  return null;
}

export function filterActivity(rows: readonly ActivityRow[], filter: ActivityGroup, query = "", channelId = "", dir: ActivityDirectory = EMPTY_DIRECTORY): ActivityRow[] {
  const q = query.trim().toLowerCase();
  return rows.filter((r) => {
    if (filter !== "all" && groupOf(r.kind) !== filter) return false;
    if (channelId && r.channelId !== channelId) return false;
    if (!q) return true;
    const who = actorText(r, dir) ?? "";
    const hay = [describeActivity(r), threadText(r, dir), channelText(r, dir) ?? "", who, r.actorId ?? "", r.kind, `#${r.id}`].join("\n").toLowerCase();
    return hay.includes(q);
  });
}

/** Channels named by the directory, plus any channel a row mentions, as #name. */
export function activityChannels(rows: readonly ActivityRow[], dir: ActivityDirectory): Array<{ id: string; label: string }> {
  const ids = new Set<string>();
  for (const id of dir.channels.keys()) ids.add(id);
  for (const r of rows) if (r.channelId) ids.add(r.channelId);
  const out = [...ids].map((id) => {
    const fromDir = (dir.channels.get(id) ?? "").trim().replace(/^#/, "");
    const fromRow = rows.find((r) => r.channelId === id)?.channelName?.trim().replace(/^#/, "") ?? "";
    const name = fromDir || fromRow;
    return { id, label: name ? `#${name}` : `#${id.slice(0, 8)}` };
  });
  return out.sort((a, b) => a.label.localeCompare(b.label) || a.id.localeCompare(b.id));
}

/**
 * The directory after a workspace read. `previous` is only the directory of
 * the workspace already on screen: a new workspace passes null, so the last
 * room's thread titles cannot ride along.
 */
export function directoryAfterLoad(
  previous: ActivityDirectory | null,
  loaded: ActivityDirectory,
  rows: readonly ActivityRow[],
): ActivityDirectory {
  const threads = new Map(loaded.threads);
  if (previous) for (const [id, title] of previous.threads) if (!threads.has(id)) threads.set(id, title);
  return absorbPayloadNames({ threads, channels: new Map(loaded.channels), actors: new Map(loaded.actors) }, rows);
}

/**
 * What a row says happened. Known kinds are sentences a person can read;
 * anything else is the kind in sentence case, so a new event is never a raw slug.
 */
const KIND_WORDS: Record<string, string> = {
  workspace_created: "Workspace created",
  member_joined: "Member joined",
  member_frozen: "Member frozen",
  member_unfrozen: "Member unfrozen",
  member_display_name_changed: "Display name changed",
  channel_created: "Channel created",
  thread_created: "Task created",
  thread_state_changed: "State changed",
  thread_assignment_changed: "Assignment changed",
  thread_ready: "Task ready",
  thread_result_set: "Result posted",
  thread_cancelled: "Task cancelled",
  thread_blocked: "Task blocked",
  blocked_resolved: "Block cleared",
  thread_landed: "Task landed",
  approval_requested: "Approval requested",
  approval_dismissed: "Approval dismissed",
  review_submitted: "Review submitted",
  message_posted: "Message posted",
  message_edited: "Message edited",
  message_tombstoned: "Message removed",
  mention_recorded: "Mention recorded",
  vote_cast: "Vote cast",
  vote_retracted: "Vote retracted",
  reaction_added: "Reaction added",
  reaction_removed: "Reaction removed",
  message_pinned: "Message pinned",
  message_unpinned: "Message unpinned",
  claim_expired: "Claim expired",
  claim_failed: "Claim failed",
  claim_stalled: "Claim stalled",
  claim_unacknowledged: "Claim unacknowledged",
  evidence_added: "Evidence added",
  status_declared: "Status updated",
};

function sentence(words: string): string {
  const t = words.trim();
  if (!t) return "Event";
  return t.charAt(0).toUpperCase() + t.slice(1);
}

/** "in_review" reads "in review" inside "Moved to in review". */
export function humanState(state: string): string {
  return state.replace(/_/g, " ").trim();
}

export function describeKind(kind: string): string {
  return KIND_WORDS[kind] ?? sentence(kind.replace(/_/g, " "));
}

/** The row's sentence. A transition with a destination says where the task went. */
export function describeActivity(row: Pick<ActivityRow, "kind" | "toState">): string {
  if (row.kind === "thread_state_changed" && row.toState) return `Moved to ${humanState(row.toState)}`;
  return describeKind(row.kind);
}

/** A display name, or @handle when that is all the member record has. The id prefix is not a name. */
export function actorFromParts(id: string, name: string | null, handle: string | null): string {
  const handleClean = handle?.trim().replace(/^@/, "") ?? "";
  const display = name?.trim() ?? "";
  if (display && display !== id.slice(0, 8) && (!handleClean || display !== handleClean)) return display;
  if (handleClean) return `@${handleClean}`;
  return id.slice(0, 8);
}

/** What the member list contributes. Null when it would only repeat the id prefix. */
export function actorWords(m: { id: string; name: string; handle?: string | null }): string | null {
  const label = actorFromParts(m.id, m.name, m.handle ?? null);
  return label === m.id.slice(0, 8) ? null : label;
}

export function threadText(row: Pick<ActivityRow, "threadId" | "title">, dir: ActivityDirectory): string {
  if (!row.threadId) return "";
  const title = dir.threads.get(row.threadId)?.trim() || row.title?.trim();
  return title || `thread ${row.threadId.slice(0, 8)}`;
}

export function channelText(row: Pick<ActivityRow, "channelId" | "channelName">, dir: ActivityDirectory): string | null {
  if (!row.channelId) return null;
  const name = (dir.channels.get(row.channelId) ?? row.channelName ?? "").trim().replace(/^#/, "");
  return name ? `#${name}` : `channel ${row.channelId.slice(0, 8)}`;
}

export function actorText(row: Pick<ActivityRow, "actorId" | "actorName" | "actorHandle">, dir: ActivityDirectory): string | null {
  if (!row.actorId) return null;
  const known = dir.actors.get(row.actorId);
  if (known) return known;
  if (row.actorName || row.actorHandle) return actorFromParts(row.actorId, row.actorName, row.actorHandle);
  return row.actorId.slice(0, 8);
}

/**
 * Fold titles, channel names and actor names carried on the events into the
 * directory, without overwriting a name the workspace list already resolved.
 */
export function absorbPayloadNames(dir: ActivityDirectory, rows: readonly ActivityRow[]): ActivityDirectory {
  const threads = new Map(dir.threads);
  const channels = new Map(dir.channels);
  const actors = new Map(dir.actors);
  let changed = false;
  for (const r of rows) {
    const title = r.title?.trim();
    if (r.threadId && title && !threads.has(r.threadId)) {
      threads.set(r.threadId, title);
      changed = true;
    }
    const channel = r.channelName?.trim();
    if (r.channelId && channel && !channels.has(r.channelId)) {
      channels.set(r.channelId, channel);
      changed = true;
    }
    if (r.actorId && !actors.has(r.actorId) && (r.actorName || r.actorHandle)) {
      const label = actorFromParts(r.actorId, r.actorName, r.actorHandle);
      if (label !== r.actorId.slice(0, 8)) {
        actors.set(r.actorId, label);
        changed = true;
      }
    }
  }
  return changed ? { threads, channels, actors } : dir;
}
