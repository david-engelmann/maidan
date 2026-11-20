// Advice on a hand-off from a decision model: what the server answers, and
// the words the console puts on it. The model makes ten yes/no judgments and
// the server's rules draw green, amber or red from them; nothing here decides
// anything, and the sheet's own buttons stay the only way to approve.
import { apiPath, failure, guarded, headers, writeInit, ApiError, type Credentials, type CredentialMode } from "./api.ts";

export type Land = "green" | "amber" | "red";
export type Answer = "yes" | "no" | "unsettled";

export type Judgment = {
  id: string;
  label: string;
  /** The model's probability that the answer is yes. */
  probability: number;
  answer: Answer;
  /** The answer as a statement, such as "Reports no passing tests". */
  reading?: string;
  /** Whether this judgment kept the advice from green. */
  concern: boolean;
};

export type Advice = {
  provider: string;
  model: string;
  land: Land;
  reasons: string[];
  judgments: Judgment[];
  key_source: "workspace" | "instance";
  cached: boolean;
};

export type AdvisorStatus = {
  enabled: boolean;
  key_source?: "workspace" | "instance" | null;
  provider?: string | null;
  model?: string | null;
  host?: string | null;
};

/** What the sheet shows in place of advice, and why. */
export type AdviceState =
  | { kind: "loading" }
  | { kind: "off" }
  | { kind: "no-key" }
  | { kind: "none" }
  | { kind: "ready"; advice: Advice }
  | { kind: "failed"; message: string };

export class AdviceError extends ApiError {}

type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

/**
 * The advice in two or three words. Each says what the model read, not what
 * to do: "ready" and "hold" read as the panel approving or blocking.
 */
export const LAND_TITLE: Record<Land, string> = {
  green: "Nothing flagged",
  amber: "Worth a look",
  red: "Problem reported",
};

/** One line under the title when nothing kept the advice from green. */
export const ALL_CLEAR = "None of the ten judgments raised a concern.";

/** Why the advice is not green, one sentence a reason. */
const REASON: Record<string, string> = {
  unresolved_failure: "It reports something still failing.",
  different_work: "It reads as work on something other than this task.",
  not_shown_done: "It does not clearly show that the task was done.",
  left_undone: "It says part of the task was left undone.",
  asks_person: "It asks a person a question first.",
  beyond_task: "It changes something the task did not ask for.",
  changes_access: "It changes sign-in or permissions.",
  deletes_data: "It deletes stored data.",
  flags_risk: "It carries a warning from the agent about a risk.",
  tests_not_reported: "The task asks for a code change, and no passing tests are reported.",
};

export function reasonText(reason: string): string {
  return REASON[reason] ?? reason.replace(/_/g, " ");
}

/** A probability as a whole percentage, never "100%" for less than certain. */
export function percent(p: number): string {
  const clamped = Math.min(1, Math.max(0, Number.isFinite(p) ? p : 0));
  const whole = Math.round(clamped * 100);
  if (whole === 100 && clamped < 1) return "99%";
  if (whole === 0 && clamped > 0) return "1%";
  return `${whole}%`;
}

/** What a judgment says, as a statement. A server from before `reading` gets the label and the answer. */
export function readingOf(j: Judgment): string {
  if (j.reading) return j.reading;
  const word = j.answer === "yes" ? "yes" : j.answer === "no" ? "no" : "unclear";
  return `${j.label}: ${word}`;
}

/**
 * How sure the model is of the statement shown: the probability of the answer
 * it reads as, so "Reports no passing tests" at 2% yes is 98% sure. An
 * unsettled judgment is not a statement the model stands behind.
 */
export function sureness(j: Judgment): string {
  if (j.answer === "unsettled") return "unsure";
  return `${percent(j.answer === "yes" ? j.probability : 1 - j.probability)} sure`;
}

/** Whether a row is clear, a concern, or one the model could not settle. */
export function rowMark(j: Judgment): "clear" | "concern" | "unsure" {
  if (j.answer === "unsettled") return "unsure";
  return j.concern ? "concern" : "clear";
}

/** A judgment as one spoken line: "Reports no passing tests, 98% sure". */
export function judgmentLine(j: Judgment): string {
  return `${readingOf(j)}, ${sureness(j)}`;
}

/** How many of the judgments kept the advice from green. */
export function concernCount(advice: Advice): number {
  return advice.judgments.filter((j) => j.concern).length;
}

/** The provider and model as the panel's small print: "Jev 1.14.0 by TypeSafe". */
export function modelLine(advice: Pick<Advice, "provider" | "model">): string {
  const jev = /^jev-(.+)$/.exec(advice.model);
  const model = jev ? `Jev ${jev[1]}` : advice.model;
  return advice.provider === "typesafe" ? `${model} by TypeSafe` : `${model} by ${advice.provider}`;
}

export class AdviceApi {
  private readonly fetchFn: Fetch;
  private readonly creds: () => Credentials;
  private readonly mode: () => CredentialMode | null;

  constructor(fetchFn: Fetch, creds: () => Credentials, mode: () => CredentialMode | null) {
    this.fetchFn = guarded(fetchFn, AdviceError);
    this.creds = creds;
    this.mode = mode;
  }

  /** Whether this instance offers advice, and whose key this workspace would use. */
  async status(workspaceId: string): Promise<AdvisorStatus> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspaceId}/land-gate/advisor`), { headers: headers(c), credentials: "include" });
    // A server from before the advisor has no such route.
    if (res.status === 404) return { enabled: false };
    if (!res.ok) return failure(res, this.mode(), AdviceError);
    return (await res.json()) as AdvisorStatus;
  }

  /**
   * Advice on a task's hand-off. The states that are not failures come back
   * as states: advice is off here, the workspace has no key, or the task has
   * no result to read yet.
   */
  async advise(threadId: string, refresh = false): Promise<AdviceState> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/threads/${threadId}/land-gate/advice`), {
      method: "POST",
      headers: headers(c, true),
      credentials: "include",
      body: JSON.stringify(refresh ? { refresh: true } : {}),
    });
    if (res.status === 404) return { kind: "off" };
    if (res.status === 409) {
      const detail = await res.json().then((b: { detail?: unknown }) => (typeof b.detail === "string" ? b.detail : ""), () => "");
      return detail.includes("key") ? { kind: "no-key" } : { kind: "none" };
    }
    if (!res.ok) return failure(res, this.mode(), AdviceError, (status) => (status === 502 ? "The model did not answer. The task is unchanged." : null));
    return { kind: "ready", advice: (await res.json()) as Advice };
  }

  /** Store this workspace's own key. The server asks the provider to accept it first. */
  async setKey(workspaceId: string, apiKey: string): Promise<AdvisorStatus> {
    const c = this.creds();
    const res = await this.fetchFn(
      apiPath(c, `/workspaces/${workspaceId}/land-gate/advisor`),
      writeInit({ method: "PUT", headers: headers(c, true), credentials: "include", body: JSON.stringify({ api_key: apiKey }) }),
    );
    if (!res.ok) return failure(res, this.mode(), AdviceError, (status, detail) => (status === 400 && detail ? detail : null));
    return (await res.json()) as AdvisorStatus;
  }

  /** Forget this workspace's key. */
  async clearKey(workspaceId: string): Promise<AdvisorStatus> {
    const c = this.creds();
    const res = await this.fetchFn(
      apiPath(c, `/workspaces/${workspaceId}/land-gate/advisor`),
      writeInit({ method: "DELETE", headers: headers(c), credentials: "include" }),
    );
    if (!res.ok) return failure(res, this.mode(), AdviceError);
    return (await res.json()) as AdvisorStatus;
  }
}
