// Where a workspace's results go: the egress allowlist that authorizes a
// destination, and how each result's delivery to one came out. Plain data in,
// plain data out, so the Deliveries tab's words are tested without a page.

export type EgressSurface = "github" | "github_branch" | "slack" | "linear";

/** A blessed destination, as `GET /workspaces/{wid}/egress-targets` lists it. */
export type EgressTarget = {
  id: string;
  surface: string;
  selector: string;
  created_at: string;
  created_by?: string | null;
  /** Why the entry can no longer allow anything; absent while it is live. */
  dead?: string | null;
};

/** One result's delivery to one target. */
export type ResultDelivery = {
  id: string;
  thread_id: string;
  surface: string;
  selector: string;
  status: string;
  attempts: number;
  last_error?: string | null;
  pr_url?: string | null;
  pr_number?: number | null;
  created_at: string;
  updated_at: string;
};

export const EGRESS_SURFACES: ReadonlyArray<{ id: EgressSurface; label: string; field: string; example: string }> = [
  { id: "github", label: "GitHub repository", field: "Repository", example: "owner/name" },
  { id: "github_branch", label: "GitHub branch", field: "Repository and base", example: "owner/name@main" },
  { id: "slack", label: "Slack channel", field: "Channel id", example: "C0123ABCDEF" },
  { id: "linear", label: "Linear team", field: "Team id", example: "Team UUID" },
];

const SURFACE_WORDS: Record<string, string> = {
  github: "GitHub",
  github_branch: "GitHub branch",
  slack: "Slack",
  linear: "Linear",
};

/** A surface by name. One this console does not know is shown as stored. */
export function surfaceWord(surface: string): string {
  return SURFACE_WORDS[surface] ?? surface;
}

export function isSurface(value: string): value is EgressSurface {
  return EGRESS_SURFACES.some((s) => s.id === value);
}

// The same character sets the server accepts, so a shape the server would
// take is never refused here. The server still has the last word: a `prod`
// base or an agent branch passes this check and is refused there.
const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const LINEAR_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/**
 * Why a selector cannot be blessed on this surface, before anything is sent,
 * or null when its shape is right. Each refusal says what to type instead.
 */
export function selectorRefusal(surface: EgressSurface, raw: string): string | null {
  const selector = raw.trim();
  if (!selector) return "Say which destination to allow.";
  switch (surface) {
    case "github":
      if (selector.includes("#")) return "Allow the repository, owner/name, without an issue number. One entry covers every pull request in it.";
      if (selector.includes("@")) return "A GitHub repository entry has no branch. To let a change open pull requests, choose GitHub branch.";
      return REPO.test(selector) ? null : "A repository is owner/name, like acme/widgets.";
    case "github_branch": {
      const at = selector.indexOf("@");
      if (at < 0) return "A GitHub branch entry is owner/name@base: the repository, and the one base its change pull requests may target.";
      if (!REPO.test(selector.slice(0, at))) return "Start with the repository, owner/name, then @ and the base branch.";
      return selector.slice(at + 1).trim() ? null : "End with the base branch, like @main.";
    }
    case "slack":
      if (selector.startsWith("#")) return "Use the channel id, which starts with C or G, not its #name. A name can move to another channel.";
      return /^[CG][A-Za-z0-9]+$/.test(selector) ? null : "A Slack channel id starts with C or G, like C0123ABCDEF.";
    case "linear":
      return LINEAR_ID.test(selector) ? null : "A Linear team id is its UUID, in lower case, not its key like ENG. A key can be renamed.";
  }
}

/** The status filter's choices, in the order the filter shows them. */
export const RESULT_FILTERS = ["all", "failed", "skipped", "pending", "delivered"] as const;
export type ResultFilter = (typeof RESULT_FILTERS)[number];

/** The query string for the workspace's result deliveries. */
export function resultDeliveryQuery(filter: ResultFilter, limit = 50): string {
  return filter === "all" ? `limit=${limit}` : `status=${filter}&limit=${limit}`;
}

const STATUS_WORDS: Record<string, string> = {
  all: "All",
  pending: "Pending",
  delivered: "Delivered",
  failed: "Failed",
  skipped: "Skipped",
};

/** A status in a word. An unknown status, from a newer server, is capitalized. */
export function statusWord(status: string): string {
  return STATUS_WORDS[status] ?? (status ? status.charAt(0).toUpperCase() + status.slice(1) : "Unknown");
}

/** The chip's tone: delivered reads as done, failed as trouble, the rest plain. */
export function statusTone(status: string): "active" | "revoked" | "" {
  if (status === "delivered") return "active";
  if (status === "failed") return "revoked";
  return "";
}

/** Only a target that was not reached is worth sending again. */
export function canReplay(d: Pick<ResultDelivery, "status">): boolean {
  return d.status === "failed" || d.status === "skipped";
}

/** Where a delivery went, as a person reads it. */
export function deliveryTarget(d: Pick<ResultDelivery, "surface" | "selector">): string {
  return `${surfaceWord(d.surface)} · ${d.selector}`;
}

/**
 * What a person needs to know about a delivery that did not land, or null.
 * A skip is not an error: it names why the target was passed over.
 */
export function deliveryReason(d: Pick<ResultDelivery, "status" | "attempts" | "last_error">): string | null {
  const error = d.last_error?.trim() || null;
  if (d.status === "failed") return error ?? "The delivery gave up after its retries.";
  if (d.status === "skipped") return error ?? "Not delivered on purpose.";
  if (d.status === "pending" && d.attempts > 0 && error) return `Trying again after: ${error}`;
  return null;
}

/** The notice after a replay, from the row the server sent back. */
export function replayOutcome(after: Pick<ResultDelivery, "status" | "surface" | "selector" | "last_error">): { text: string; ok: boolean } {
  const target = deliveryTarget(after);
  const why = after.last_error?.trim();
  if (after.status === "pending") return { text: `Sending again to ${target}`, ok: true };
  if (after.status === "delivered") return { text: `Delivered to ${target}`, ok: true };
  if (after.status === "skipped") return { text: `Still skipped: ${why || "the target is not allowed"}`, ok: false };
  return { text: `Still ${statusWord(after.status).toLowerCase()}${why ? `: ${why}` : ""}`, ok: false };
}

/** The list with one row swapped for the server's newer copy, in place. */
export function withRow(rows: readonly ResultDelivery[], row: ResultDelivery): ResultDelivery[] {
  return rows.map((r) => (r.id === row.id ? row : r));
}

/**
 * Whether the allowlist is worth asking for. A token, and a session made from
 * one, is exactly its grant, so one without token:admin is told so with no
 * request the browser would log as refused. A person's own sign-in carries no
 * token:admin of its own (the server checks what they hold), so only the
 * server can answer for it.
 */
export function asksForAllowlist(tokenBacked: boolean, grant: { capabilities: string[] } | null): boolean {
  if (!tokenBacked) return true;
  return Boolean(grant?.capabilities.includes("token:admin"));
}

/** Said in place of the allowlist to someone who may not see it. */
export const ALLOWLIST_NEED = "Only a workspace admin sees where results may be delivered. Ask one to allow a destination.";
