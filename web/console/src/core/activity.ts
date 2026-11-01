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
};

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

function actorOf(o: Record<string, unknown>): string | null {
  const p = (o.payload && typeof o.payload === "object" ? (o.payload as Record<string, unknown>) : {}) as Record<string, unknown>;
  for (const k of ["actor_id", "member_id", "author_id", "by"]) {
    const v = str(o[k]) ?? str(p[k]);
    if (v) return v;
  }
  return null;
}

/** A row from GET /workspaces/{id}/events or a live event frame; null if it has no id. */
export function toActivity(o: Record<string, unknown>): ActivityRow | null {
  const id = typeof o.id === "number" ? o.id : typeof o.log_id === "number" ? o.log_id : null;
  if (id === null) return null;
  const p = (o.payload && typeof o.payload === "object" ? (o.payload as Record<string, unknown>) : {}) as Record<string, unknown>;
  return {
    id,
    kind: str(o.kind) ?? "event",
    threadId: str(o.thread_id) ?? str(p.thread_id),
    channelId: str(o.channel_id) ?? str(p.channel_id),
    actorId: actorOf(o),
    at: str(o.occurred_at) ?? str(p.occurred_at),
  };
}

/** Newest first, one row per id, at most `cap`. */
export function mergeActivity(rows: ActivityRow[], incoming: ActivityRow[], cap = ACTIVITY_CAP): ActivityRow[] {
  const by = new Map<number, ActivityRow>();
  for (const r of rows) by.set(r.id, r);
  for (const r of incoming) by.set(r.id, r);
  return [...by.values()].sort((a, b) => b.id - a.id).slice(0, cap);
}

export function groupOf(kind: string): Exclude<ActivityGroup, "all"> | null {
  if (kind.startsWith("vote_")) return "votes";
  if (kind.startsWith("message_") || kind.startsWith("reaction_")) return "messages";
  if (kind.startsWith("thread_") || kind.startsWith("task_")) return "tasks";
  if (kind.startsWith("member_") || kind.startsWith("agent_") || kind.startsWith("token_")) return "members";
  return null;
}

export function filterActivity(rows: ActivityRow[], group: ActivityGroup): ActivityRow[] {
  return group === "all" ? rows : rows.filter((r) => groupOf(r.kind) === group);
}

/** "thread_state_changed" reads "thread state changed". */
export function describeKind(kind: string): string {
  return kind.replace(/_/g, " ");
}
