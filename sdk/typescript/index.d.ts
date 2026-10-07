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
  /**
   * Retries after a failure in transit, 408, 429 (honouring `Retry-After`),
   * 500/502/503/504, or a 409 `idempotency-key-in-flight`. Default 2; 0 turns
   * retries off. Writes carry one `Idempotency-Key` across their attempts.
   */
  maxRetries?: number;
  /** Test seam: how the client waits between attempts. */
  sleep?: (ms: number) => Promise<void>;
}

/** A fresh `Idempotency-Key` (a UUID). */
export declare function newIdempotencyKey(): string;

/** Delay before retry `attempt` (0-based): `Retry-After` if sent, else 0.5s·2^n capped at 8s, jittered. */
export declare function retryDelayMs(
  attempt: number,
  retryAfterHeader?: string | null,
  random?: () => number,
): number;

/** Parse `Maidan-Room-LSN`. Rejects WAL text so this is never a Consistency-Token. */
export declare function parseRoomLsn(value: string | null | undefined): number | undefined;

/** Observable `$type` for an event kind (`message_posted` → `maidan.event.message_posted/1`). */
export declare function eventType(kind: string): string;

/** An RFC 9457 problem body as the server sends it. Unknown members are kept. */
export interface Problem {
  type: string;
  title: string;
  status: number;
  detail: string;
  /** Set on a cursor-too-old 409: refetch, never clamp. */
  must_refetch?: boolean;
  /** On a cursor-too-old 409: the snapshot path covering the pruned prefix. */
  snapshot?: string | null;
  [member: string]: unknown;
}

/** URI prefix of every problem `type` the server emits. */
export declare const PROBLEM_BASE: "https://maidan.dev/problems/";

/** The most rows the server returns for one page; the paging helpers ask for no more. */
export declare const MAX_PAGE_SIZE: 500;

/**
 * A failed request. Each problem `type` the server documents has a subclass
 * (catch with `instanceof`); an unknown type is {@link UnknownProblemError}.
 */
export declare class MaidanError extends Error {
  constructor(status: number, problem?: unknown, message?: string);
  status: number;
  /** The problem `type` URI; unset when the body was not a problem. */
  type?: string;
  title?: string;
  /** The problem's `detail`, or the body's text when it was not a problem. */
  detail?: string;
  /** The problem body as sent, unknown members included. */
  problem?: Problem;
  /** Seconds from `Retry-After` (sent on 429 and 503). */
  retryAfter?: number;
  get isConflict(): boolean; // 409
  /** 409 + must_refetch / cursor-too-old — fail loud, never clamp. */
  get isCursorTooOld(): boolean;
  get isForbidden(): boolean; // 403 (missing capability / channel access — not retryable)
  get isRateLimited(): boolean; // 429
}
/** 404 `not-found`. */
export declare class NotFoundError extends MaidanError {}
/** 405 `method-not-allowed`. */
export declare class MethodNotAllowedError extends MaidanError {}
/** 409 `conflict`: the resource's state refuses the change. */
export declare class ConflictError extends MaidanError {}
/** 400 `bad-request`. */
export declare class BadRequestError extends MaidanError {}
/** 401 `unauthorized`: missing or invalid bearer token. */
export declare class UnauthorizedError extends MaidanError {}
/** 401 `invalid-signature` (webhook ingress). */
export declare class InvalidSignatureError extends MaidanError {}
/** 403 `forbidden`: a missing capability or channel access. Not retryable. */
export declare class ForbiddenError extends MaidanError {}
/** 413 `payload-too-large`. */
export declare class PayloadTooLargeError extends MaidanError {}
/** 415 `unsupported-media-type`. */
export declare class UnsupportedMediaTypeError extends MaidanError {}
/** 429 `rate-limited`; see `retryAfter`. */
export declare class RateLimitedError extends MaidanError {}
/** 502 `bad-gateway`. */
export declare class BadGatewayError extends MaidanError {}
/** 500 `internal`. */
export declare class InternalError extends MaidanError {}
/** 503 `overloaded`: refused without running; retry after `retryAfter`. */
export declare class OverloadedError extends MaidanError {}
/** 422 `idempotency-key-reused`: the key was used for a different request. */
export declare class IdempotencyKeyReusedError extends MaidanError {}
/** 409 `idempotency-key-in-flight`: the first request with the key still runs. */
export declare class IdempotencyKeyInFlightError extends MaidanError {}
/** 409 `cursor-too-old`: refetch from `snapshot`, never clamp the cursor. */
export declare class CursorTooOldError extends MaidanError {
  get snapshot(): string | undefined;
}
/** 409 `event-log-broken`: the hash chain failed verification. */
export declare class EventLogBrokenError extends MaidanError {}
/** A problem `type` this client does not know, or a body that is not a problem. */
export declare class UnknownProblemError extends MaidanError {}

/** Problem `type` URI → error class. */
export declare const PROBLEM_TYPES: Readonly<Record<string, typeof MaidanError>>;
/** The error for a failed response: the subclass its problem `type` names. */
export declare function problemError(status: number, body: unknown): MaidanError;

// Response models, from the server's OpenAPI schemas. Timestamps are RFC 3339
// strings. A string enum lists the values the server sends today and still
// accepts others, so a new value does not break a caller. Objects may carry
// members added after this client was published.

export type ThreadState = "open" | "in_review" | "closed" | "archived" | (string & {});
export type MemberKind = "human" | "agent" | (string & {});
export type ArtifactKind =
  | "screenshot"
  | "recording"
  | "transcript"
  | "code_dump"
  | "attachment"
  | "context_snapshot"
  | (string & {});
export type ImportMode = "new" | "restore" | (string & {});
export type RefSide = "thread" | "message" | (string & {});
export type ReviewDecision = "approve" | "request_changes" | (string & {});

export interface Workspace {
  id: WorkspaceId;
  name: string;
  created_at: string;
  updated_at: string;
  tombstoned_at?: string | null;
}

export interface ImportResult {
  workspace_id: WorkspaceId;
  mode: ImportMode;
}

export interface Member {
  id: MemberId;
  workspace_id: WorkspaceId;
  handle: string;
  kind: MemberKind;
  display_name?: string | null;
  created_at: string;
  updated_at: string;
  tombstoned_at?: string | null;
}

export interface TokenQuota {
  capability: string;
  max_per_window: number;
  window_secs: number;
}

/** A mint's answer. `secret` is returned here once and never again. */
export interface MintedToken {
  id: string;
  secret: string;
  workspace_id: WorkspaceId;
  member_id: MemberId;
  capabilities: string[];
  expires_at?: string | null;
  quotas: TokenQuota[];
}

/** Token metadata; never carries the secret. */
export interface TokenSummary {
  id: string;
  workspace_id: WorkspaceId;
  member_id: MemberId;
  label?: string | null;
  capabilities: string[];
  created_at: string;
  expires_at?: string | null;
  revoked_at?: string | null;
}

export interface Channel {
  id: ChannelId;
  workspace_id: WorkspaceId;
  name: string;
  private: boolean;
  topic?: string | null;
  created_at: string;
  updated_at: string;
  tombstoned_at?: string | null;
}

export interface Thread {
  id: ThreadId;
  channel_id: ChannelId;
  parent_thread_id?: ThreadId | null;
  title?: string | null;
  state: ThreadState;
  assignee_id?: MemberId | null;
  owner_id?: MemberId | null;
  assignment_expires_at?: string | null;
  /** The fencing token {@link Client.renewClaim} takes. */
  claim_lease_id?: string | null;
  work_started_at?: string | null;
  created_at: string;
  updated_at: string;
  tombstoned_at?: string | null;
}

/** A content-addressed pin (`maidan:event/{id}` + its hash). */
export interface StrongRef {
  uri: string;
  content_hash: string;
}

/** A claim: the thread's fields at the top level, plus the pin. */
export interface ClaimedThread extends Thread {
  pin: StrongRef;
}

export interface ThreadResult {
  thread_id: ThreadId;
  /** The producer's JSON, as it was set. */
  result: unknown;
  produced_by: MemberId;
  produced_at: string;
}

/** A structured message block, discriminated by `type`. */
export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "code"; code: string; language?: string | null }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error?: boolean }
  | { type: "resource_link"; uri: string; mime_type?: string | null; title?: string | null }
  | { type: string & {}; [member: string]: unknown };

export interface Message {
  id: string;
  thread_id: ThreadId;
  author_id: MemberId;
  body: string;
  content?: ContentBlock[] | null;
  metadata?: unknown;
  posted_at: string;
  edited_at?: string | null;
  tombstoned_at?: string | null;
}

export interface Artifact {
  id: string;
  sha256: Sha256;
  size_bytes: number;
  kind: ArtifactKind;
  mime_type?: string | null;
  uploaded_by?: MemberId | null;
  created_at: string;
  tombstoned_at?: string | null;
}

export interface MessageEditView {
  id: number;
  message_id: string;
  editor_id: MemberId;
  edited_at: string;
  /** Present only with `include_edits=true`. */
  body_before?: string | null;
  body_after?: string | null;
}

export interface Reference {
  id: string;
  src_kind: RefSide;
  src_id: string;
  dst_kind: RefSide;
  dst_id: string;
  relation: string;
  created_at: string;
}

export interface ThreadTransition {
  id: string;
  thread_id: ThreadId;
  from_state: ThreadState;
  to_state: ThreadState;
  actor_id: MemberId;
  occurred_at: string;
}

export interface ThreadBrief {
  title?: string | null;
  parent_thread_id?: ThreadId | null;
  owner_id?: MemberId | null;
  required_skills?: string[];
  created_at: string;
}

export interface AcceptedDecision {
  thread_id: ThreadId;
  state: ThreadState;
  title?: string | null;
  produced_by: MemberId;
  produced_at: string;
  result_kind?: string | null;
  status?: string | null;
  summary?: string | null;
}

export interface ThreadReview {
  thread_id: ThreadId;
  reviewer_id: MemberId;
  actor_id?: MemberId | null;
  decision: ReviewDecision;
  note?: string | null;
  dismissed_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface GlossaryTerm {
  id: string;
  workspace_id: WorkspaceId;
  term: string;
  definition: string;
  aliases: string[];
  created_by: MemberId;
  created_at: string;
  updated_at: string;
}

export interface PackElision {
  elided_message_count: number;
  elided_token_estimate: number;
  first_elided_id: string;
  last_elided_id: string;
  summary: string;
}

export interface ParentGrounding {
  thread_id: ThreadId;
  state: ThreadState;
  title?: string | null;
  opening_message?: Message | null;
  latest_result?: unknown;
}

/** GET /threads/{id}/context: one canonical pack. Stable fields, then the tail. */
export interface ThreadContext {
  workspace_id: WorkspaceId;
  channel_id: ChannelId;
  thread_id: ThreadId;
  thread: ThreadBrief;
  messages: Message[];
  message_edits: MessageEditView[];
  references: Reference[];
  artifacts: Artifact[];
  transitions: ThreadTransition[];
  state: ThreadState;
  updated_at: string;
  prefix_sha256: string;
  prefix_bytes: number;
  glossary?: GlossaryTerm[];
  accepted_decisions?: AcceptedDecision[];
  change_requests?: ThreadReview[];
  assignee_id?: MemberId | null;
  assignment_expires_at?: string | null;
  claim_lease_id?: string | null;
  work_started_at?: string | null;
  elision?: PackElision | null;
  parent_grounding?: ParentGrounding | null;
  as_of?: number | null;
  next_message_cursor?: string | null;
}

/** A row of GET /workspaces/{id}/events. `payload` is the event, shaped by `kind`. */
export interface StoredEvent {
  $type: string;
  id: number;
  lsn: number;
  kind: string;
  workspace_id?: WorkspaceId | null;
  channel_id?: ChannelId | null;
  thread_id?: ThreadId | null;
  payload: unknown;
  occurred_at: string;
  prev_hash: string;
  content_hash: string;
  content_key?: string;
  traceparent?: string | null;
  tracestate?: string | null;
}

/** Projector shape for {@link Client.follow}. */
export interface FollowSpec {
  workspaceId: string;
  channelId?: string;
  threadId?: string;
  types?: string[];
  consumerId?: string;
  afterId?: number;
  /** Backfill page size: default 100, at most {@link MAX_PAGE_SIZE}. */
  pageLimit?: number;
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
  /** Last seen `Maidan-Room-LSN` (event-log high-water). Not a WAL token. */
  lastRoomLsn?: number;
  maxRetries: number;

  constructor(baseUrl?: string, token?: string, options?: ClientOptions);

  workspaces: {
    create(name: string): Promise<Workspace>;
    get(id: WorkspaceId): Promise<Workspace>;
    /** Admin-only (`token:admin`). `bundle` is a signed `maidan.workspace.export/1` envelope. */
    import(bundle: unknown, mode?: "new" | "restore"): Promise<ImportResult>;
    /** GET /workspaces/{id}/events — projector-shaped HTTP backfill. */
    events(id: WorkspaceId, query?: Record<string, string | number>): Promise<StoredEvent[]>;
    /** Every event after `query.after_id`, fetching `query.limit` (default 100, at most {@link MAX_PAGE_SIZE}) per page. */
    eventsAll(id: WorkspaceId, query?: Record<string, string | number>): AsyncGenerator<StoredEvent>;
  };
  members: {
    create(wid: WorkspaceId, handle: string, kind?: MemberKind, displayName?: string): Promise<Member>;
    list(wid: WorkspaceId): Promise<Member[]>;
  };

  tokens: {
    mint(
      wid: WorkspaceId,
      mid: MemberId,
      capabilities?: string[],
      opts?: { label?: string; capabilitySet?: string; expiresAt?: string },
    ): Promise<MintedToken>;
    list(wid: WorkspaceId, mid: MemberId): Promise<TokenSummary[]>;
  };

  channels: {
    list(wid: WorkspaceId): Promise<Channel[]>;
    create(wid: WorkspaceId, name: string, priv?: boolean): Promise<Channel>;
  };
  threads: {
    /** GET /channels/{cid}/threads — one page (`limit`, `cursor` = last thread id). */
    list(cid: ChannelId, query?: { limit?: number; cursor?: ThreadId }): Promise<Thread[]>;
    /** Every live thread in the channel, `pageSize` (default 100, at most {@link MAX_PAGE_SIZE}) per request. */
    listAll(cid: ChannelId, opts?: { pageSize?: number }): AsyncGenerator<Thread>;
    create(cid: ChannelId, title: string): Promise<Thread>;
    get(id: ThreadId): Promise<Thread>;
    context(id: ThreadId, query?: Record<string, string | number | boolean>): Promise<ThreadContext>;
    /** `body.action`: `start_review`, `close` or `archive`. */
    transition(id: ThreadId, body: { action: string }): Promise<Thread>;
    setResult(id: ThreadId, result: unknown): Promise<ThreadResult>;
    getResult(id: ThreadId): Promise<ThreadResult>;
  };
  messages: {
    list(tid: ThreadId, query?: Record<string, string | number>): Promise<Message[]>;
    post(tid: ThreadId, body: string): Promise<Message>;
  };
  artifacts: {
    upload(bytes: Uint8Array | ArrayBuffer | string, kind: ArtifactKind): Promise<Artifact>;
    get(sha: Sha256): Promise<Uint8Array>;
    meta(sha: Sha256): Promise<Artifact>;
  };

  /** Hero: readiness/skill/lease-aware claim of the next thread in a channel. */
  claimNextThread(cid: ChannelId, body?: { lease_secs?: number }): Promise<ClaimedThread | null>;
  /** Holder-only lease heartbeat. */
  renewClaim(
    id: ThreadId,
    claimLeaseId: string,
    leaseSecs?: number,
  ): Promise<Thread>;

  subscribe(
    filter: Record<string, unknown>,
    onEvent: (event: EventFrame) => void,
    onError?: (err: unknown) => void,
    opts?: { afterId?: number; consumerId?: string },
  ): Promise<Subscription>;

  /** HTTP backfill then WS cutover. A pruned cursor throws {@link CursorTooOldError}. */
  follow(
    spec: FollowSpec,
    onEvent: (event: EventFrame) => void,
    onError?: (err: unknown) => void,
  ): Promise<Subscription>;

  /** Wait helpers wrap `subscribe`; resolve with the event or null on timeout. */
  waitForResult(threadId: ThreadId, workspaceId: WorkspaceId, timeoutMs?: number): Promise<EventFrame | null>;
  waitForMention(memberId: MemberId, workspaceId: WorkspaceId, timeoutMs?: number): Promise<EventFrame | null>;
  waitForReady(workspaceId: WorkspaceId, channelId?: ChannelId, timeoutMs?: number): Promise<EventFrame | null>;
}

/** The ledger's token tiers. `input` is uncached input on every provider. */
export interface TokenUsage {
  input: number;
  output: number;
  cache_read: number;
  cache_write_5m: number;
  cache_write_1h: number;
}

/** Micro-USD per million tokens, one rate per tier, snapshotted per report. */
export interface PriceSnapshot {
  input_usd_micros_per_million: number;
  output_usd_micros_per_million: number;
  cache_read_usd_micros_per_million: number;
  cache_write_5m_usd_micros_per_million: number;
  cache_write_1h_usd_micros_per_million: number;
}

/** What a normalizer can tell from the response; the rest is the caller's. */
export interface UsageEvidence {
  provider: string;
  service_tier?: string;
  cache_miss_reason?: string;
}

/** The economic part of a `report_usage` body. */
export interface NormalizedUsage {
  model: string;
  tokens: TokenUsage;
  evidence: UsageEvidence;
}

export type UsageProvider =
  | "anthropic"
  | "bedrock-converse"
  | "openai-responses"
  | "openai-chat"
  | "gemini"
  | "deepseek"
  | "mistral"
  | "xai"
  | "vllm";

/** A usage object that cannot be read into the ledger's shape. */
export class UsageError extends Error {}

export const USAGE_PROVIDERS: readonly UsageProvider[];

/**
 * Turn one provider response into the ledger's tokens. `model` names the model
 * when the response does not (Bedrock Converse); `provider` overrides the
 * evidence provider name.
 */
export function normalizeUsage(
  provider: UsageProvider,
  response: Record<string, unknown>,
  options?: { model?: string; provider?: string },
): NormalizedUsage;

/** `ceil(sum(tokens x rate) / 1_000_000)`, as the ledger checks it. */
export function usdMicros(tokens: TokenUsage, priceSnapshot: PriceSnapshot): number;

export default Client;
