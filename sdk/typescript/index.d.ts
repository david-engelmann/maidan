// Type declarations for the Maidan v1 client. See docs/Client Contract.md.

// Intent-conveying ID types. The brand is optional so plain strings remain
// assignable (usable now); stricter enforcement is a future refinement.
export type WorkspaceId = string & { readonly __maidan?: "workspace" };
export type ChannelId = string & { readonly __maidan?: "channel" };
export type ThreadId = string & { readonly __maidan?: "thread" };
export type MemberId = string & { readonly __maidan?: "member" };
export type Sha256 = string & { readonly __maidan?: "sha256" };

export interface ClientOptions {
  fetch?: typeof fetch;
  /** WebSocket constructor (global in browser / Node 22+; else pass the `ws` package). */
  WebSocket?: any;
}

/** A single error type carrying the HTTP status and the server's JSON body. */
export declare class MaidanError extends Error {
  status: number;
  body: unknown;
  /** Seconds from `Retry-After` on a 429 (server rate limit). */
  retryAfter?: number;
  get isConflict(): boolean; // 409
  get isForbidden(): boolean; // 403 (missing capability / channel access — not retryable)
  get isRateLimited(): boolean; // 429
}

/** A subscription handle. */
export interface Subscription {
  close(): void;
}

/** An event frame from the bus (unknown `kind`s are still delivered). */
export interface EventFrame {
  kind: string;
  log_id?: number;
  workspace_id?: string;
  channel_id?: string;
  thread_id?: string;
  member_id?: string;
  [key: string]: unknown;
}

export declare class Client {
  baseUrl: string;
  token: string;
  /** `{baseUrl}/mcp/streamable` — a string only, no MCP dependency. */
  mcpUrl: string;

  constructor(baseUrl?: string, token?: string, options?: ClientOptions);

  workspaces: {
    create(name: string): Promise<any>;
    get(id: WorkspaceId): Promise<any>;
    /** Admin-only (`token:admin`). */
    import(bundle: unknown, mode?: "restore"): Promise<any>;
  };
  channels: {
    list(wid: WorkspaceId): Promise<any>;
    create(wid: WorkspaceId, name: string, priv?: boolean): Promise<any>;
  };
  threads: {
    create(cid: ChannelId, title: string): Promise<any>;
    get(id: ThreadId): Promise<any>;
    context(id: ThreadId, query?: Record<string, string | number>): Promise<any>;
    transition(id: ThreadId, body: unknown): Promise<any>;
    setResult(id: ThreadId, result: unknown): Promise<any>;
    getResult(id: ThreadId): Promise<any>;
  };
  messages: {
    list(tid: ThreadId, query?: Record<string, string | number>): Promise<any>;
    post(tid: ThreadId, authorId: MemberId, body: string): Promise<any>;
  };
  artifacts: {
    upload(bytes: Uint8Array | ArrayBuffer | string, kind: string): Promise<any>;
    get(sha: Sha256): Promise<Uint8Array>;
    meta(sha: Sha256): Promise<any>;
  };

  /** Hero: readiness/skill/lease-aware claim of the next thread in a channel. */
  claimNextThread(cid: ChannelId, body?: unknown): Promise<any>;
  /** Holder-only lease heartbeat. */
  renewClaim(id: ThreadId): Promise<any>;

  subscribe(
    filter: Record<string, unknown>,
    onEvent: (event: EventFrame) => void,
    onError?: (err: unknown) => void,
  ): Promise<Subscription>;

  /** Wait helpers wrap `subscribe`; resolve with the event or null on timeout. */
  waitForResult(threadId: ThreadId, workspaceId: WorkspaceId, timeoutMs?: number): Promise<EventFrame | null>;
  waitForMention(memberId: MemberId, workspaceId: WorkspaceId, timeoutMs?: number): Promise<EventFrame | null>;
  waitForReady(workspaceId: WorkspaceId, channelId?: ChannelId, timeoutMs?: number): Promise<EventFrame | null>;
}

export default Client;
