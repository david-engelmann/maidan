// How the console reaches the server. A port of the board's api.js with the
// DOM reads taken out: the credential state is passed in, so it can be tested.

/** What the page holds right now: a token pasted in this tab, or a session. */
export type Credentials = {
  /** The API origin, with no trailing slash. */
  base: string;
  /** A token kept in this tab only, until the server trades it for a cookie. */
  pastedToken: string;
  /** The session cookie was made from a pasted token. */
  tokenSession: boolean;
  /** The member a signed-in session is pinned to, if any. */
  sessionMemberId: string | null;
  /** The token a token-made session runs on, so the console can say "this page" by it. Never the secret. */
  sessionTokenId?: string | null;
};

export const STORAGE = {
  token: "maidan_token",
  workspace: "maidan_workspace",
  wsResume: "maidan_ws_resume",
} as const;

export function trimBase(base: string): string {
  return base.replace(/\/$/, "");
}

/** Whether the page acts with a token's authority. */
export function hasToken(c: Credentials): boolean {
  return Boolean(c.pastedToken.trim()) || c.tokenSession;
}

export function uiPath(c: Credentials, suffix: string): string {
  return `${trimBase(c.base)}/ui/api${suffix}`;
}

/**
 * A read or a write. Token authority goes to the bearer API; a signed-in
 * session goes through the session-authed /ui/api proxy (same suffix).
 */
export function apiPath(c: Credentials, suffix: string): string {
  return hasToken(c) ? `${trimBase(c.base)}${suffix}` : uiPath(c, suffix);
}

export function wsUrl(base: string): string {
  const trimmed = trimBase(base);
  const origin = trimmed || (typeof window !== "undefined" && window.location?.origin ? window.location.origin : "");
  return origin.replace(/^http/, "ws") + "/ws/subscribe";
}

export function headers(c: Credentials, json = false): Record<string, string> {
  const h: Record<string, string> = { Accept: "application/json" };
  if (json) h["Content-Type"] = "application/json";
  const t = c.pastedToken.trim();
  if (t) h.Authorization = `Bearer ${t}`;
  return h;
}

export const SIGN_IN_FIRST = "Sign in first: paste a token under Connect this browser.";
export const SIGN_IN_FIRST_OIDC =
  "Sign in first: paste a token under Connect this browser, or sign in with your identity provider.";

/** Why a write cannot go out, or null when it can. */
export function writeRefusal(c: Credentials, oidcLoginPath: string | null): string | null {
  if (hasToken(c) || c.sessionMemberId) return null;
  return oidcLoginPath ? SIGN_IN_FIRST_OIDC : SIGN_IN_FIRST;
}

/** Why a bearer-only route cannot be called, or null when it can. */
export function bearerRefusal(c: Credentials): string | null {
  if (hasToken(c)) return null;
  return c.sessionMemberId
    ? "This needs a bearer token. A signed-in session cannot call it."
    : "Set a bearer token to do this.";
}

export function idempotencyKey(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/**
 * A mutating request. Each call carries a fresh Idempotency-Key, so a
 * duplicated delivery replays the first response instead of writing twice.
 */
export function writeInit(init: RequestInit = {}, key = idempotencyKey()): RequestInit {
  return { ...init, headers: { ...(init.headers as Record<string, string> | undefined), "Idempotency-Key": key } };
}

export type CredentialMode = "session" | "bearer" | "delegated";

/** The badge word for each credential, shown once a credential works. */
export const MODE_WORD: Record<CredentialMode, string> = {
  session: "Session",
  bearer: "Bearer token",
  delegated: "Delegated token",
};

/** One line on what the credential is allowed to stand for. */
export const MODE_LINE: Record<CredentialMode, string> = {
  session: "Session: your own sign-in, acting as you",
  bearer: "Bearer token: acts as this member",
  delegated: "Delegated token: acts only within the grant it was lent",
};

/** A string the server meant a person to read, including `{ error: { message } }`. */
export function problemDetail(body: unknown): string | null {
  if (!body || typeof body !== "object") return null;
  const rec = body as { message?: unknown; detail?: unknown; error?: unknown };
  const nested = rec.error && typeof rec.error === "object" ? (rec.error as { message?: unknown; detail?: unknown }) : null;
  for (const value of [rec.message, rec.detail, rec.error, nested?.message, nested?.detail]) {
    if (typeof value === "string" && value.trim()) return value;
  }
  return null;
}

/** What GET /me says the page acts with. */
export function credentialMode(
  me: { is_bearer: boolean; delegation_grant_id?: string | null } | null,
): CredentialMode | null {
  if (!me) return null;
  if (!me.is_bearer) return "session";
  return me.delegation_grant_id ? "delegated" : "bearer";
}

/**
 * A refusal in words. The server's detail is read only so a missing
 * capability can be named; it is never shown, and neither is the status.
 */
export function humanError(status: number, detail: string | null, mode: CredentialMode | null): string {
  const needs = /(?:capability|needs?|requires?)[^a-z]*([a-z]+:[a-z_]+)/i.exec(detail ?? "");
  if (status === 401) {
    if (mode === "session") return "Your session was not accepted; it may have ended. Sign in again";
    if (mode === "delegated") return "This delegated token was not accepted; it or its grant may have ended. Sign in again with a working one";
    if (mode === "bearer") return "Your token was not accepted. Sign in again with a working one";
    return "Your token or session was not accepted. Sign in again";
  }
  if (status === 403) return refusal(needs ? needs[1] : null, mode);
  if (status === 404) return "Not found. It may have been deleted, or it belongs to another workspace";
  if (status === 409) return CONFLICT;
  if (status === 413) return "That is too large for the server to accept";
  if (status === 429) return "Too many requests. Wait a moment, then try again";
  if (status === 0) return OFFLINE;
  if (status === 502 || status === 503 || status === 504) return UNAVAILABLE;
  if (status >= 500) return SERVER_ERROR;
  return "The request was not accepted";
}

/** A 409: the thing changed under the request, so the page is behind. */
export const CONFLICT = "Someone changed this first. Reload and try again";

/** A 5xx with no better word. A hosted customer has no server log to read. */
export const SERVER_ERROR = "The server hit an error. Try again; if it keeps failing, contact your workspace admin or support";

/** 502, 503 and 504: the server is up behind a proxy but not answering yet, often mid-deploy. */
export const UNAVAILABLE = "The server is busy or restarting. Try again in a moment";

/**
 * Where token:admin comes from. People mints only for a credential that
 * already holds it, so the first one comes from Tools, then Connect.
 */
export const ADMIN_TOKEN_PATH =
  "Ask a workspace admin for a token that has it. If you created this workspace, mint an admin token in Tools, then Connect, then sign out and sign in with it";

function refusal(cap: string | null, mode: CredentialMode | null): string {
  const who = mode === "session" ? "Your session" : mode === "delegated" ? "This delegated token" : "Your token";
  const lead = cap ? `${who} is not allowed to do this; it needs ${cap}.` : `${who} is not allowed to do this.`;
  if (cap === "token:admin") return `${lead} ${ADMIN_TOKEN_PATH}`;
  if (cap === "approval:grant")
    return `${lead} Accepting a gate needs you signed in through your identity provider, or a token an admin granted approval:grant. You can still decline or cancel it`;
  if (mode === "session")
    return cap
      ? `${lead} Sign out, then sign in with a token that has it; a workspace admin mints one in People`
      : `${lead} Sign out, then sign in with a token that has the right capability`;
  if (mode === "delegated")
    return cap
      ? `${lead} It holds only what its grant lends: ask for a grant that includes it`
      : `${lead} It holds only what its grant lends: ask for a grant with the right capability`;
  return cap ? `${lead} Mint a token with it in Tools, then Tokens` : `${lead} Mint one with the right capability in Tools, then Tokens`;
}

/**
 * Whether signing out posts /auth/logout. Discovery that never answered
 * (null) is not proof there is no cookie, so it posts.
 */
export function signOutPostsLogout(sessions: boolean | null | undefined, cachedMemberId: string | null): boolean {
  if (sessions === true) return true;
  if (sessions === false) return Boolean(cachedMemberId);
  return true;
}

export const OFFLINE = "Could not reach the server. Check your connection, then try again";
export const SIGNED_OUT = "Your sign-in ended or was revoked. Sign in again to carry on.";

/** A refused or failed request, with the status and any Retry-After kept for the caller. */
export class ApiError extends Error {
  readonly status: number;
  readonly retryAfter: number | null;
  constructor(message: string, status = 0, retryAfter: number | null = null) {
    super(message);
    this.status = status;
    this.retryAfter = retryAfter;
  }
}

/** Seconds to wait from a Retry-After header (seconds or an HTTP date), or null. */
export function retryAfterSeconds(value: string | null | undefined, now = Date.now()): number | null {
  if (!value) return null;
  const t = value.trim();
  if (/^\d+$/.test(t)) {
    const n = Number(t);
    return Number.isFinite(n) ? n : null;
  }
  const at = Date.parse(t);
  return Number.isNaN(at) ? null : Math.max(0, Math.ceil((at - now) / 1000));
}

function waitWords(secs: number): string {
  if (secs < 60) return `${secs} second${secs === 1 ? "" : "s"}`;
  const m = Math.ceil(secs / 60);
  return `${m} minute${m === 1 ? "" : "s"}`;
}

/** Heard when the server says this tab's credential no longer works (any 401). */
const authLost = new Set<() => void>();
/** Bumped whenever this tab's credential changes, so a 401 for an older one is ignored. */
let authEpoch = 0;
const sentUnder = new WeakMap<Response, number>();
export function bumpAuthEpoch(): void {
  authEpoch++;
}
/** A 401 on this response says nothing about the tab's credential (a cookie-only route). */
export function keepAuth(res: Response): void {
  sentUnder.set(res, -1);
}
/** A 401 for this tab's current credential signs the tab out. For a module
 * that words its own failures instead of going through `failure()`. */
export function noteAuthLost(res: Response): void {
  if (res.status === 401 && (sentUnder.get(res) ?? authEpoch) === authEpoch) authLost.forEach((fn) => fn());
}
export function onAuthLost(fn: () => void): () => void {
  authLost.add(fn);
  return () => authLost.delete(fn);
}

/**
 * The one way a screen's request fails. It reads the server's detail, keeps
 * Retry-After, words the refusal, and on a 401 tells the session so the page
 * goes back to sign-in. `own` returns the server's sentence where a module
 * shows it (a refused move), or null for the standard words.
 */
export async function failure(
  res: Response,
  mode: CredentialMode | null,
  Err: new (message: string, status?: number, retryAfter?: number | null) => ApiError = ApiError,
  own: (status: number, detail: string | null) => string | null = () => null,
): Promise<never> {
  // The status alone says the credential is gone; a body that never finishes
  // must not keep the tab signed in.
  noteAuthLost(res);
  let detail: string | null = null;
  try {
    detail = problemDetail(await res.json());
  } catch {
    detail = null;
  }
  const retry = res.status === 429 || res.status === 503 ? retryAfterSeconds(res.headers.get("Retry-After")) : null;
  const mine = own(res.status, typeof detail === "string" ? detail : null);
  let text = mine ?? humanError(res.status, detail, mode);
  if (retry !== null && retry > 0) text = res.status === 429 ? `Too many requests. Try again in ${waitWords(retry)}` : mine === null && res.status === 503 ? `The server is busy or restarting. Try again in ${waitWords(retry)}` : `${text}. Try again in ${waitWords(retry)}`;
  throw new Err(text, res.status, retry);
}

/** A fetch that never rejects with a raw TypeError: an unreachable server reads as OFFLINE. */
export function guarded(fetchFn: (url: string, init?: RequestInit) => Promise<Response>, Err: new (message: string, status?: number) => ApiError = ApiError) {
  return async (url: string, init?: RequestInit): Promise<Response> => {
    const epoch = authEpoch;
    try {
      const res = await fetchFn(url, init);
      if (res && typeof res === "object" && !sentUnder.has(res)) sentUnder.set(res, epoch);
      return res;
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") throw e;
      throw new Err(OFFLINE, 0);
    }
  };
}
