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

export type ProviderId = "typesafe" | "openai" | "google" | "perplexity" | "cloudflare";

/** One provider of decision models, as this workspace sees it. */
export type AdvisorProvider = {
  id: ProviderId;
  name: string;
  model: string;
  host: string;
  key_source?: "workspace" | "instance" | null;
};

export type AdvisorStatus = {
  enabled: boolean;
  /** Whether the server can keep a key for a workspace. Absent from a server that does not say, which is read as yes. */
  can_store_key?: boolean;
  /** These four describe the provider a call that names none would ask. */
  key_source?: "workspace" | "instance" | null;
  provider?: string | null;
  model?: string | null;
  host?: string | null;
  /** Every provider the server can ask. */
  providers?: AdvisorProvider[];
};

/** The provider's key, as its own site calls it. */
export const KEY_NAME: Record<ProviderId, string> = {
  typesafe: "TypeSafe API key",
  openai: "OpenAI API key",
  google: "Gemini API key",
  perplexity: "Perplexity API key",
  cloudflare: "Cloudflare Workers AI token",
};

/**
 * Cloudflare's key is two things: a token, and the id of the account whose
 * Workers AI it calls. Every other provider's is the key alone.
 */
export const needsAccount = (provider: ProviderId): boolean => provider === "cloudflare";

/** Said in place of the key forms on a server that has no encryption key to keep one with. */
export const KEY_CANNOT_STORE = "This server cannot store a decision model key. Its operator has not set an encryption key.";

/** The providers a call from this workspace can ask now. */
export function usableProviders(status: AdvisorStatus | null): AdvisorProvider[] {
  return (status?.providers ?? []).filter((p) => p.key_source);
}

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
export const ALL_CLEAR = "All ten readings are clear.";

/** Why the advice is not green, one sentence a reason, for a reading the model settled. */
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

/**
 * The same sentence when the model could not settle the judgment behind it.
 * An unsettled warning sign keeps the advice from green, and the summary says
 * what is true of it: the model is unsure, not that the sign is there.
 */
const UNSETTLED_REASON: Record<string, string> = {
  unresolved_failure: "It is unclear whether something is still failing.",
  left_undone: "It is unclear whether part of the task was left undone.",
  asks_person: "It is unclear whether it asks a person a question first.",
  beyond_task: "It is unclear whether it changes something the task did not ask for.",
  changes_access: "It is unclear whether it changes sign-in or permissions.",
  deletes_data: "It is unclear whether it deletes stored data.",
  flags_risk: "It is unclear whether the agent warns about a risk.",
};

/** The tests reason rests on two judgments, and either may be the unsettled one. */
function testsReason(unsure: { codeChange: boolean; tests: boolean }): string {
  if (unsure.codeChange && unsure.tests) return "It is unclear whether the task asks for a code change and whether passing tests are reported.";
  if (unsure.codeChange) return "It is unclear whether the task asks for a code change, and no passing tests are reported.";
  if (unsure.tests) return "The task asks for a code change, and it is unclear whether passing tests are reported.";
  return REASON.tests_not_reported;
}

/**
 * One sentence for why the advice is not green. `judgments` are the ones the
 * response carries: with them, a reason that rests on an unsettled judgment
 * says it is unclear, and without them the sentence is the settled one.
 * "Does not clearly show the task was done" is true of an unsettled reading
 * as it is of a settled no, so it stands for both.
 */
export function reasonText(reason: string, judgments: Judgment[] = []): string {
  const unsettled = (id: string) => judgments.some((j) => j.id === id && j.answer === "unsettled");
  if (reason === "tests_not_reported") return testsReason({ codeChange: unsettled("code_change"), tests: unsettled("tests_passed") });
  if (unsettled(reason)) return UNSETTLED_REASON[reason] ?? REASON[reason] ?? reason.replace(/_/g, " ");
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

/**
 * Whether a row is clear, a concern, or a concern because the model could not
 * settle it. Only what kept the advice from green is marked: an unsettled
 * answer the rules did not need (tests on a task that asks for no code change)
 * is clear.
 */
export function rowMark(j: Judgment): "clear" | "concern" | "unsure" {
  if (!j.concern) return "clear";
  return j.answer === "unsettled" ? "unsure" : "concern";
}

/** A judgment as one spoken line: "Reports no passing tests, 98% sure". */
export function judgmentLine(j: Judgment): string {
  return `${readingOf(j)}, ${sureness(j)}`;
}

/** How many of the judgments kept the advice from green. */
export function concernCount(advice: Advice): number {
  return advice.judgments.filter((j) => j.concern).length;
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
  async advise(threadId: string, refresh = false, provider: ProviderId | null = null): Promise<AdviceState> {
    const c = this.creds();
    // The path and its method share a line: check:spec reads the method off the path's line.
    const body = JSON.stringify({ ...(refresh ? { refresh: true } : {}), ...(provider ? { provider } : {}) });
    const res = await this.fetchFn(apiPath(c, `/threads/${threadId}/land-gate/advice`), { method: "POST", headers: headers(c, true), credentials: "include", body });
    if (res.status === 404) return { kind: "off" };
    if (res.status === 409) {
      const detail = await res.json().then((b: { detail?: unknown }) => (typeof b.detail === "string" ? b.detail : ""), () => "");
      return detail.includes("key") ? { kind: "no-key" } : { kind: "none" };
    }
    if (!res.ok) return failure(res, this.mode(), AdviceError, (status) => (status === 502 ? "The model did not answer. The task is unchanged." : null));
    return { kind: "ready", advice: (await res.json()) as Advice };
  }

  /** Store this workspace's own key for a provider. The server asks the provider to accept it first. */
  async setKey(workspaceId: string, apiKey: string, provider: ProviderId = "typesafe", accountId: string | null = null): Promise<AdvisorStatus> {
    const c = this.creds();
    const body = JSON.stringify({ api_key: apiKey, provider, ...(accountId ? { account_id: accountId } : {}) });
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspaceId}/land-gate/advisor`), writeInit({ method: "PUT", headers: headers(c, true), credentials: "include", body }));
    // A refused key (400) and an instance that cannot keep one (409) say why in the server's words.
    if (!res.ok) return failure(res, this.mode(), AdviceError, (status, detail) => ((status === 400 || status === 409) && detail ? detail : null));
    return (await res.json()) as AdvisorStatus;
  }

  /** Forget this workspace's key for a provider. */
  async clearKey(workspaceId: string, provider: ProviderId = "typesafe"): Promise<AdvisorStatus> {
    const c = this.creds();
    const res = await this.fetchFn(apiPath(c, `/workspaces/${workspaceId}/land-gate/advisor`) + `?provider=${provider}`, writeInit({ method: "DELETE", headers: headers(c), credentials: "include" }));
    if (!res.ok) return failure(res, this.mode(), AdviceError);
    return (await res.json()) as AdvisorStatus;
  }
}
