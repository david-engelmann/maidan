// The console's sign-in state, as a small store the useSession hook reads.
// A port of the board's session.js: discovery, the cookie session, the
// token-for-session exchange, and /me, without touching the DOM.
import { credentialMode, type CredentialMode, type Credentials, headers, writeInit } from "./api.ts";

export type Discovery = {
  /** False when discovery has not been answered yet (unreachable, 5xx, 429). */
  answered: boolean;
  oidcLoginPath: string | null;
  sessions: boolean | null;
  signupPath: string | null;
  turnstileSiteKey: string | null;
};

export type SessionState = {
  status: "loading" | "signed-out" | "signed-in";
  credentials: Credentials;
  memberId: string | null;
  displayName: string | null;
  workspaceId: string | null;
  mode: CredentialMode | null;
  discovery: Discovery;
  error: string | null;
};

export type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

const NO_DISCOVERY: Discovery = { answered: false, oidcLoginPath: null, sessions: null, signupPath: null, turnstileSiteKey: null };

export function initialSession(base: string): SessionState {
  return {
    status: "loading",
    credentials: { base, pastedToken: "", tokenSession: false, sessionMemberId: null },
    memberId: null,
    displayName: null,
    workspaceId: null,
    mode: null,
    discovery: NO_DISCOVERY,
    error: null,
  };
}

export async function readDiscovery(fetchFn: Fetch, base: string, timeoutMs = 4000): Promise<Discovery> {
  try {
    const res = await fetchFn(`${base}/.well-known/maidan.json`, { signal: AbortSignal.timeout(timeoutMs) });
    if (res.ok) {
      const auth = ((await res.json()) as { auth?: Record<string, unknown> }).auth ?? {};
      return {
        answered: true,
        oidcLoginPath: auth.oidc && typeof auth.oidc_login === "string" ? auth.oidc_login : null,
        sessions: typeof auth.sessions === "boolean" ? auth.sessions : null,
        signupPath: typeof auth.oidc_signup === "string" ? auth.oidc_signup : null,
        turnstileSiteKey: typeof auth.turnstile_site_key === "string" ? auth.turnstile_site_key : null,
      };
    }
    // No discovery document: a token is the only path offered.
    if (res.status < 500 && res.status !== 429) return { ...NO_DISCOVERY, answered: true };
  } catch {
    /* unreachable, timed out, or not JSON */
  }
  return NO_DISCOVERY;
}

type SessionBody = { member_id: string; token_id?: string | null; display_name?: string | null; workspace_id: string };

/**
 * A store with one writer per field and a generation counter, so an answer
 * that a newer paste or read has overtaken never lands (a slow rejection of
 * token A must not sign out token B).
 */
export class SessionStore {
  private state: SessionState;
  private listeners = new Set<() => void>();
  private gen = 0;
  private readonly fetchFn: Fetch;

  constructor(base: string, fetchFn: Fetch) {
    this.state = initialSession(base);
    this.fetchFn = fetchFn;
  }

  getSnapshot = (): SessionState => this.state;

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  private set(patch: Partial<SessionState>): void {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((fn) => fn());
  }

  private get base(): string {
    return this.state.credentials.base;
  }

  async start(): Promise<void> {
    const discovery = await readDiscovery(this.fetchFn, this.base);
    this.set({ discovery });
    await this.refresh();
  }

  /** Read the cookie session, then /me for the credential mode. */
  async refresh(): Promise<SessionState> {
    const gen = ++this.gen;
    try {
      const res = await this.fetchFn(`${this.base}/auth/session`, { credentials: "include" });
      if (gen !== this.gen) return this.state;
      if (!res.ok) {
        const creds = { ...this.state.credentials, tokenSession: false, sessionMemberId: null, sessionTokenId: null };
        const signedIn = Boolean(creds.pastedToken);
        this.set({ credentials: creds, status: signedIn ? "signed-in" : "signed-out", memberId: signedIn ? this.state.memberId : null, displayName: null });
        if (signedIn) await this.readMe(gen);
        return this.state;
      }
      const body = (await res.json()) as SessionBody;
      if (gen !== this.gen) return this.state;
      const shown = typeof body.display_name === "string" ? body.display_name.trim() : "";
      this.set({
        status: "signed-in",
        credentials: { ...this.state.credentials, tokenSession: Boolean(body.token_id), sessionMemberId: body.member_id, sessionTokenId: typeof body.token_id === "string" ? body.token_id : null },
        memberId: body.member_id,
        displayName: shown || null,
        workspaceId: this.state.workspaceId ?? body.workspace_id,
        error: null,
      });
      await this.readMe(gen);
    } catch {
      if (gen === this.gen) this.set({ status: "signed-out", error: "Could not reach the server" });
    }
    return this.state;
  }

  private async readMe(gen: number): Promise<void> {
    try {
      const res = await this.fetchFn(`${this.base}/me`, { headers: headers(this.state.credentials), credentials: "include" });
      if (gen !== this.gen || !res.ok) return;
      const me = (await res.json()) as { member_id: string; workspace_id: string; is_bearer: boolean; delegation_grant_id?: string | null };
      if (gen !== this.gen) return;
      this.set({ mode: credentialMode(me), memberId: me.member_id, workspaceId: this.state.workspaceId ?? me.workspace_id });
    } catch {
      /* the session read already said who this is */
    }
  }

  /**
   * Trade a pasted token for an HttpOnly session with its authority. A server
   * with no session key (404) cannot hold one, so the token stays in this
   * tab's memory only, never in storage.
   */
  async signInWithToken(secret: string): Promise<{ ok: boolean; error?: string }> {
    const gen = ++this.gen;
    let res: Response;
    try {
      res = await this.fetchFn(
        `${this.base}/auth/session/from-token`,
        writeInit({ method: "POST", headers: { Accept: "application/json", Authorization: `Bearer ${secret}` }, credentials: "include" }),
      );
    } catch {
      return { ok: false, error: "Could not reach the server" };
    }
    if (gen !== this.gen) return { ok: false };
    if (res.status === 404) {
      this.set({ credentials: { ...this.state.credentials, pastedToken: secret } });
      await this.refresh();
      return { ok: true };
    }
    if (!res.ok) {
      return {
        ok: false,
        error:
          res.status === 401
            ? "That token was not accepted: check it was copied whole and has not expired or been revoked, or mint a new one in Tokens."
            : "Could not sign in with that token",
      };
    }
    this.set({ credentials: { ...this.state.credentials, pastedToken: "", tokenSession: true } });
    await this.refresh();
    return { ok: true };
  }

  setWorkspace(workspaceId: string): void {
    this.set({ workspaceId });
  }

  /** Forget this tab's credentials; the caller posts /auth/logout when due. */
  signOut(): void {
    this.gen++;
    this.set({
      status: "signed-out",
      credentials: { ...this.state.credentials, pastedToken: "", tokenSession: false, sessionMemberId: null },
      memberId: null,
      displayName: null,
      mode: null,
    });
  }
}
