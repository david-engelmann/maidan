// Every kind of event a webhook can subscribe to, grouped by family for the
// form. The names are the server's own `EventKind` (the OpenAPI spec's enum,
// which test/eventKinds.test.ts holds this list to in both directions). The
// server accepts any of them for a webhook: its create route refuses only a
// name it does not know, so nothing is left out here.

export type EventGroup = { id: string; title: string; kinds: readonly string[] };

export const EVENT_GROUPS: readonly EventGroup[] = [
  { id: "workspace", title: "Workspace and channels", kinds: ["workspace_created", "channel_created"] },
  {
    id: "threads",
    title: "Tasks and threads",
    kinds: [
      "thread_created",
      "thread_state_changed",
      "thread_assignment_changed",
      "thread_ready",
      "thread_result_set",
      "thread_cancelled",
      "thread_attributes_set",
      "thread_blocked",
      "blocked_resolved",
      "thread_landed",
      "thread_notice",
      "thread_handoff",
      "thread_spawn_denied",
      "status_declared",
      "wait_timed_out",
      "schedule_skipped",
    ],
  },
  {
    id: "messages",
    title: "Messages",
    kinds: ["message_posted", "message_edited", "message_tombstoned", "mention_recorded", "reaction_added", "reaction_removed", "message_pinned", "message_unpinned", "reference_added"],
  },
  {
    id: "reviews",
    title: "Reviews and approvals",
    kinds: ["review_submitted", "evidence_added", "approval_requested", "approval_resolved", "approval_dismissed", "vote_cast", "vote_retracted"],
  },
  { id: "claims", title: "Claims", kinds: ["claim_expired", "claim_unacknowledged", "claim_stalled", "claim_failed"] },
  { id: "members", title: "Members", kinds: ["member_joined", "member_frozen", "member_unfrozen", "member_display_name_changed"] },
  { id: "intake", title: "Intake", kinds: ["intake_proposed", "intake_accepted", "intake_rejected"] },
  { id: "usage", title: "Usage and credits", kinds: ["usage_reported", "usage_over_allowance", "credits_deposited"] },
  { id: "delivery", title: "Delivery and integrations", kinds: ["result_delivery_failed", "projector_misconfigured"] },
  { id: "files", title: "Files and memory", kinds: ["artifact_upserted", "memory_block_updated"] },
];

/** Every kind, in the groups' order. */
export const ALL_EVENT_KINDS: readonly string[] = EVENT_GROUPS.flatMap((g) => g.kinds);

// A kind in words is its name with spaces and a capital. These are the ones
// that read better written out: "tombstoned" is a word for the log, and an
// upsert is an add or an update.
const LABELS: Record<string, string> = {
  message_tombstoned: "Message removed",
  artifact_upserted: "File added or updated",
  mention_recorded: "Member mentioned",
};

/** A kind a person can read: `thread_result_set` is "Thread result set". */
export function eventLabel(kind: string): string {
  const own = LABELS[kind];
  if (own) return own;
  const words = kind.replace(/_/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : kind;
}

/** How many of a group's kinds are chosen. */
export function chosenIn(group: EventGroup, chosen: readonly string[]): number {
  return group.kinds.filter((k) => chosen.includes(k)).length;
}

/** The chosen kinds with a whole group switched on or off. */
export function withGroup(group: EventGroup, chosen: readonly string[], on: boolean): string[] {
  const rest = chosen.filter((k) => !group.kinds.includes(k));
  return on ? [...rest, ...group.kinds] : rest;
}

/** The chosen kinds in the groups' order, which is the order they are sent in. */
export function inOrder(chosen: readonly string[]): string[] {
  return ALL_EVENT_KINDS.filter((k) => chosen.includes(k));
}
