// Ending a task before it lands: a cancel, or a close as abandoned, duplicate
// or superseded. Who may do which mirrors the server's rules so the console
// only offers what would be accepted; the server still decides, and its
// refusals come back as calm sentences rather than codes. Fetch is passed in,
// so the module is tested without a browser.
import { ApiError, apiPath, failure, guarded, headers, writeInit, type CredentialMode, type Credentials } from "./api.ts";
import type { Fetch, Thread } from "./board.ts";

export type EndKind = "cancel" | "abandon";

/** The server's closed set for close-abandoned. */
export type CloseReason = "abandoned" | "duplicate" | "superseded";

/** The longest cancel reason the server keeps (`CANCEL_REASON_MAX_CHARS`). */
export const CANCEL_REASON_MAX = 500;

/** What each close reason means, in the words the form offers. */
export const CLOSE_REASON_WORDS: Record<CloseReason, string> = {
  abandoned: "Nobody will do it",
  duplicate: "Another task already covers it",
  superseded: "A later task replaces it",
};

export const END_TITLE: Record<EndKind, string> = {
  cancel: "Cancel task",
  abandon: "Close as abandoned",
};

/**
 * The thread's state and the caller decide. Both routes refuse a task that
 * has ended. A cancel needs `workspace:write`, and then either
 * `thread:transition` or being the task's creator. A close-abandoned needs
 * `thread:transition`, and then being the creator, the claim holder, or a
 * maintainer (`token:admin`). Unknown capabilities offer nothing.
 */
export function endActionsFor(
  th: Pick<Thread, "state" | "assignee_id" | "created_by">,
  me: string | null,
  capabilities: readonly string[] | null,
): { cancel: boolean; abandon: boolean } {
  const live = th.state === "open" || th.state === "in_review";
  if (!live || !me || !capabilities) return { cancel: false, abandon: false };
  const has = (c: string) => capabilities.includes(c);
  const creator = th.created_by === me;
  const holder = th.assignee_id === me;
  return {
    cancel: has("workspace:write") && (has("thread:transition") || creator),
    abandon: has("thread:transition") && (has("token:admin") || creator || holder),
  };
}

/**
 * What ending the task does, from "What ending early does to partial work"
 * in docs/Swarms.md. The confirm step names each effect before anything is sent.
 */
export function endEffects(kind: EndKind, reason: CloseReason = "abandoned"): string[] {
  const claim = "The claim ends at once. Whoever holds the task is told to stop, and its next lease renewal is refused.";
  const delivery = "A change delivery that is not yet writing to GitHub is skipped. Messages are still sent.";
  const result = "A result already written is kept and stays readable.";
  if (kind === "cancel") {
    return [
      claim,
      "The task closes and is parked with your reason, so nobody is handed it again. A closed task can be archived but never reopened.",
      "Its pull request, if the change flow opened one, is taken out of the merge queue and closed with your reason as a comment. The branch is left.",
      delivery,
      "Your reason is posted on the task.",
      result,
    ];
  }
  const dependents =
    reason === "abandoned"
      ? "Tasks that depend on this one keep waiting: nothing landed."
      : "Tasks that depend on this one move to the task you name, unless that would make a loop.";
  return [
    claim,
    "The task closes, and nobody can claim it again.",
    "Its pull request, if the change flow opened one, is taken out of the merge queue and closed with the close reason as a comment. The branch is left.",
    delivery,
    dependents,
    "If the task already has a result, its review and land gates still apply, so the close can be refused until they pass.",
  ];
}

/** Why a cancel reason cannot be sent yet, or null. */
export function cancelReasonRefusal(reason: string): string | null {
  const t = reason.trim();
  if (!t) return "Say why in a few words. The person or agent holding the task reads it.";
  if ([...t].length > CANCEL_REASON_MAX) return `Keep it under ${CANCEL_REASON_MAX} characters.`;
  return null;
}

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

/** The task id in a pasted id or console link, or null. */
export function taskIdFrom(input: string): string | null {
  const m = UUID.exec(input.trim());
  return m ? m[0].toLowerCase() : null;
}

/** Why a close cannot be sent yet, or null. The other task is required for a duplicate only. */
export function closeRefusal(reason: CloseReason, other: string, self: string): string | null {
  if (reason === "abandoned") return null;
  const raw = other.trim();
  if (!raw) return reason === "duplicate" ? "Paste a link to the task that already covers it, or its id." : null;
  const id = taskIdFrom(raw);
  if (!id) return "That is not a task link or id.";
  if (id === self.toLowerCase()) return "Name a different task, not this one.";
  return null;
}

/** The close-abandoned body: `duplicate_of` only where the reason takes one. */
export function closeBody(reason: CloseReason, other: string): { reason: CloseReason; duplicate_of?: string } {
  const id = reason === "abandoned" ? null : taskIdFrom(other);
  return id ? { reason, duplicate_of: id } : { reason };
}

export class EndTaskError extends ApiError {}

/**
 * The server's refusal in calm words. A missing capability is left to the
 * standard sentence, which names it.
 */
export function endRefusal(kind: EndKind, status: number, detail: string | null): string | null {
  const d = detail ?? "";
  if (status === 403 && /capability/i.test(d)) return null;
  if (status === 403 || (status === 409 && /producer|claim holder|maintainer/i.test(d))) {
    return kind === "cancel"
      ? "Only the person who created this task, or someone allowed to move tasks, can cancel it."
      : "Only the task's creator, whoever holds it, or a workspace admin can close it as abandoned.";
  }
  if (status === 409) {
    if (/already|only an open or in-review/i.test(d)) return "This task has already ended. Reload to see where it stands.";
    if (/separation of duties/i.test(d))
      return "This task has an owner and you worked on it, so someone else closes it, or an outside reviewer approves it first.";
    if (/approv|review|land gate|refute/i.test(d))
      return "This task has a result, so it still has to pass its review before it can close. Approve it or send it back instead.";
    return d ? `This did not go through: ${d}` : null;
  }
  if (status === 400 && d) return `Check the form: ${d}`;
  return null;
}

/** The two writes behind the end actions. Paths go through apiPath, so a session uses /ui/api. */
export class EndTaskApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = guarded(fetchFn, EndTaskError);
    this.creds = creds;
    this.mode = mode;
  }

  private async post<T>(kind: EndKind, suffix: string, body: unknown): Promise<T> {
    const c = this.creds();
    const res = await this.fetchFn(
      apiPath(c, suffix),
      writeInit({ method: "POST", headers: headers(c, true), credentials: "include", body: JSON.stringify(body) }),
    );
    if (!res.ok) return failure(res, this.mode(), EndTaskError, (status, detail) => endRefusal(kind, status, detail));
    return (await res.json()) as T;
  }

  /** `POST /threads/{id}/cancel`. A repeat answers with the first cancel. */
  cancel(threadId: string, reason: string): Promise<{ thread: Thread; replayed?: boolean }> {
    const why = cancelReasonRefusal(reason);
    if (why) return Promise.reject(new EndTaskError(why, 400));
    return this.post("cancel", `/threads/${threadId}/cancel`, { reason: reason.trim() });
  }

  /** `POST /threads/{id}/close-abandoned`. */
  closeAbandoned(threadId: string, reason: CloseReason, other = ""): Promise<Thread> {
    const why = closeRefusal(reason, other, threadId);
    if (why) return Promise.reject(new EndTaskError(why, 400));
    return this.post("abandon", `/threads/${threadId}/close-abandoned`, closeBody(reason, other));
  }
}
