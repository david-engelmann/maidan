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
  return trimBase(base).replace(/^http/, "ws") + "/ws/subscribe";
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
    if (mode === "session") return "Your session was not accepted; it may have ended. Sign in again, or use Change to paste a token";
    if (mode === "delegated") return "This delegated token was not accepted; it or its grant may have ended. Use Change to set a working one";
    if (mode === "bearer") return "Your token was not accepted. Use Change to set a working one";
    return "Your token or session was not accepted. Use Change to set a working one";
  }
  if (status === 403) return refusal(needs ? needs[1] : null, mode);
  if (status === 404) return "Not found. It may have been deleted, or it belongs to another workspace";
  if (status === 409) return "Refused";
  if (status === 413) return "That is too large for the server to accept";
  if (status === 429) return "Too many requests. Wait a moment, then try again";
  if (status >= 500) return "The server hit an error. Try again; if it keeps failing, check the server log";
  return "The request was not accepted";
}

function refusal(cap: string | null, mode: CredentialMode | null): string {
  const who = mode === "session" ? "Your session" : mode === "delegated" ? "This delegated token" : "Your token";
  const lead = cap ? `${who} is not allowed to do this; it needs ${cap}.` : `${who} is not allowed to do this.`;
  if (cap === "token:admin") return `${lead} Ask a workspace admin for a token (maidan init prints the first admin token)`;
  if (cap === "approval:grant")
    return `${lead} Accepting a gate needs you signed in through your identity provider, or a token an admin granted approval:grant. You can still decline or cancel it`;
  if (mode === "session")
    return cap
      ? `${lead} A session cannot mint tokens, so use Change to paste a token that has it`
      : `${lead} Use Change to paste a token with the right capability`;
  if (mode === "delegated")
    return cap
      ? `${lead} It holds only what its grant lends: ask for a grant that includes it`
      : `${lead} It holds only what its grant lends: ask for a grant with the right capability`;
  return cap ? `${lead} Mint a token with it in Tokens` : `${lead} Mint one with the right capability in Tokens`;
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
