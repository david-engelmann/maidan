// Maidan TypeScript client (v1 surface). REST + WebSocket, dependency-free:
// uses the global `fetch` (Node 18+) and a pluggable WebSocket (global in the
// browser / Node 22+, or inject one via `options.WebSocket`). See docs/Client
// Contract.md for the frozen surface.

import { bootPrefix } from "./cache.js";

export {
  CacheError,
  bootPrefix,
  cacheKey,
  cacheKeyFields,
  cachedPrefix,
  gatewaySession,
} from "./cache.js";

/** The URI prefix of every problem `type` the server emits (RFC 9457). */
export const PROBLEM_BASE = "https://maidan.dev/problems/";

/**
 * A failed request. `type`, `title` and `detail` come from the server's RFC 9457
 * problem body; `problem` is that body as sent, unknown members included. A body
 * that is not a problem (a proxy's HTML page, say) leaves `type` unset and puts
 * its text in `detail`. Each problem type has its own subclass; a type this
 * client does not know is an {@link UnknownProblemError}.
 */
export class MaidanError extends Error {
  constructor(status, problem, message) {
    const p = problem && typeof problem === "object" ? problem : undefined;
    const detail = p ? stringOr(p.detail) : typeof problem === "string" ? problem : undefined;
    super(message || `Maidan request failed: HTTP ${status}${detail ? `: ${detail}` : ""}`);
    this.name = new.target.name;
    this.status = status;
    this.type = p ? stringOr(p.type) : undefined;
    this.title = p ? stringOr(p.title) : undefined;
    this.detail = detail;
    this.problem = p;
    // Seconds from Retry-After, sent on 429 (rate limit) and 503 (overloaded).
    this.retryAfter = undefined;
  }
  get isConflict() {
    return this.status === 409;
  }
  /** 409 + must_refetch / cursor-too-old — fail loud, never clamp. */
  get isCursorTooOld() {
    if (this.status !== 409) return false;
    if (this.problem && this.problem.must_refetch === true) return true;
    return this.type === `${PROBLEM_BASE}cursor-too-old` || this.type === "cursor_too_old";
  }
  get isForbidden() {
    return this.status === 403;
  }
  get isRateLimited() {
    return this.status === 429;
  }
}

export class NotFoundError extends MaidanError {}
export class MethodNotAllowedError extends MaidanError {}
export class ConflictError extends MaidanError {}
export class BadRequestError extends MaidanError {}
export class UnauthorizedError extends MaidanError {}
export class InvalidSignatureError extends MaidanError {}
export class ForbiddenError extends MaidanError {}
export class PayloadTooLargeError extends MaidanError {}
export class UnsupportedMediaTypeError extends MaidanError {}
export class RateLimitedError extends MaidanError {}
export class BadGatewayError extends MaidanError {}
export class InternalError extends MaidanError {}
export class OverloadedError extends MaidanError {}
export class IdempotencyKeyReusedError extends MaidanError {}
/** A retry arrived while the first request with its key still runs; retry shortly. */
export class IdempotencyKeyInFlightError extends MaidanError {}
/** The cursor is behind the retained log: refetch from `snapshot`, never clamp. */
export class CursorTooOldError extends MaidanError {
  get snapshot() {
    return this.problem ? stringOr(this.problem.snapshot) : undefined;
  }
}
/** The hash-chained event log failed verification; the server fails closed. */
export class EventLogBrokenError extends MaidanError {}
/** A problem `type` this client does not know, or a body that is not a problem. */
export class UnknownProblemError extends MaidanError {}

/** Problem `type` URI → error class, one per type the server documents. */
export const PROBLEM_TYPES = Object.freeze({
  [`${PROBLEM_BASE}not-found`]: NotFoundError,
  [`${PROBLEM_BASE}method-not-allowed`]: MethodNotAllowedError,
  [`${PROBLEM_BASE}conflict`]: ConflictError,
  [`${PROBLEM_BASE}bad-request`]: BadRequestError,
  [`${PROBLEM_BASE}unauthorized`]: UnauthorizedError,
  [`${PROBLEM_BASE}invalid-signature`]: InvalidSignatureError,
  [`${PROBLEM_BASE}forbidden`]: ForbiddenError,
  [`${PROBLEM_BASE}payload-too-large`]: PayloadTooLargeError,
  [`${PROBLEM_BASE}unsupported-media-type`]: UnsupportedMediaTypeError,
  [`${PROBLEM_BASE}rate-limited`]: RateLimitedError,
  [`${PROBLEM_BASE}bad-gateway`]: BadGatewayError,
  [`${PROBLEM_BASE}internal`]: InternalError,
  [`${PROBLEM_BASE}overloaded`]: OverloadedError,
  [`${PROBLEM_BASE}idempotency-key-reused`]: IdempotencyKeyReusedError,
  [`${PROBLEM_BASE}idempotency-key-in-flight`]: IdempotencyKeyInFlightError,
  [`${PROBLEM_BASE}cursor-too-old`]: CursorTooOldError,
  [`${PROBLEM_BASE}event-log-broken`]: EventLogBrokenError,
});

/** The error for a failed response: the subclass its problem `type` names. */
export function problemError(status, body) {
  const type = body && typeof body === "object" ? body.type : undefined;
  const Cls = (typeof type === "string" && Object.hasOwn(PROBLEM_TYPES, type) && PROBLEM_TYPES[type]) || UnknownProblemError;
  return new Cls(status, body);
}

function stringOr(v) {
  return typeof v === "string" ? v : undefined;
}

function envDefault(key) {
  return typeof process !== "undefined" && process.env ? process.env[key] : undefined;
}

/** Parse `Maidan-Room-LSN`. Rejects WAL text so this is never a Consistency-Token. */
export function parseRoomLsn(value) {
  if (value == null) return undefined;
  const trimmed = String(value).trim();
  if (!trimmed || trimmed.includes("/")) return undefined;
  if (!/^\d+$/.test(trimmed)) return undefined;
  const n = Number(trimmed);
  if (!Number.isSafeInteger(n) || n < 0) return undefined;
  return n;
}

/** Observable `$type` for an event kind (`message_posted` → `maidan.event.message_posted/1`). */
export function eventType(kind) {
  return `maidan.event.${kind}/1`;
}

/**
 * The most rows the server returns for one page: it clamps a larger `limit` to
 * this. The paging helpers ask for no more, because they stop at the first
 * short page, and a clamped page would look like the last one.
 */
export const MAX_PAGE_SIZE = 500;

/** The limit a paging helper asks for: `n`, 100 when not positive, at most MAX_PAGE_SIZE. */
function pageSize(n) {
  const v = Number(n);
  return v > 0 ? Math.min(v, MAX_PAGE_SIZE) : 100;
}

const WRITE_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);
const RETRY_STATUSES = new Set([408, 429, 500, 502, 503, 504]);
const IN_FLIGHT_TYPE = `${PROBLEM_BASE}idempotency-key-in-flight`;

/** A fresh `Idempotency-Key`: one per logical write, reused by its retries. */
export function newIdempotencyKey() {
  const c = typeof globalThis !== "undefined" ? globalThis.crypto : undefined;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  let hex = "";
  for (let i = 0; i < 32; i++) hex += Math.floor(Math.random() * 16).toString(16);
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-a${hex.slice(17, 20)}-${hex.slice(20)}`;
}

/**
 * The delay before retry `attempt` (0-based): the server's `Retry-After` when
 * it sent one, else exponential backoff from 0.5s capped at 8s, with jitter.
 */
export function retryDelayMs(attempt, retryAfterHeader, random = Math.random) {
  const ra = retryAfterHeader == null ? NaN : Number(retryAfterHeader);
  if (Number.isFinite(ra) && ra >= 0) return Math.min(ra, 60) * 1000;
  const base = Math.min(8000, 500 * 2 ** attempt);
  return base / 2 + random() * (base / 2);
}

export class Client {
  /**
   * @param {string} [baseUrl] defaults to MAIDAN_URL
   * @param {string} [token] defaults to MAIDAN_TOKEN
   * @param {{ fetch?: typeof fetch, WebSocket?: any, maxRetries?: number,
   *   sleep?: (ms: number) => Promise<void> }} [options] `maxRetries`
   *   (default 2; 0 turns retries off) bounds the retries of a request that
   *   failed in transit or answered 408, 429, 500, 502, 503 or 504, or 409
   *   in-flight for its key.
   */
  constructor(baseUrl, token, options = {}) {
    this.baseUrl = (baseUrl || envDefault("MAIDAN_URL") || "http://127.0.0.1:8080").replace(
      /\/+$/,
      "",
    );
    this.token = token || envDefault("MAIDAN_TOKEN") || "";
    this._fetch = options.fetch || (typeof fetch !== "undefined" ? fetch : undefined);
    this._WebSocket = options.WebSocket || (typeof WebSocket !== "undefined" ? WebSocket : undefined);
    this.maxRetries = options.maxRetries === undefined ? 2 : options.maxRetries;
    this._sleep = options.sleep || ((ms) => new Promise((r) => setTimeout(r, ms)));

    // MCP is a URL, not a dependency (docs/Client Contract.md §4).
    this.mcpUrl = `${this.baseUrl}/mcp/streamable`;
    /** Last seen Maidan-Room-LSN (event-log high-water). Not a WAL token. */
    this.lastRoomLsn = undefined;

    this.workspaces = {
      create: (name) => this._req("POST", "/workspaces", { name }),
      get: (id) => this._req("GET", `/workspaces/${id}`),
      import: (bundle, mode) =>
        this._req("POST", `/workspaces/import${mode ? `?mode=${mode}` : ""}`, bundle),
      events: (id, query) => this._req("GET", `/workspaces/${id}/events${qs(query)}`),
      /** Every event after `query.after_id`, page by page (`limit` per page, at most MAX_PAGE_SIZE). */
      eventsAll: (id, query = {}) => this._eventsAll(id, query),
    };
    // Provisioning. `members.create` is the unauthenticated seed route, present
    // only on a server built with the `bootstrap` feature; production turns it
    // off and provisions through `maidan init` plus `tokens.mint`.
    this.members = {
      create: (wid, handle, kind = "agent", displayName) =>
        this._req("POST", `/workspaces/${wid}/members`, {
          handle,
          kind,
          ...(displayName === undefined ? {} : { display_name: displayName }),
        }),
      list: (wid) => this._req("GET", `/workspaces/${wid}/members`),
    };
    // Needs `token:admin` — the capability `maidan init`'s admin token carries.
    // `mint` returns the secret once, in the response; `list` is metadata only.
    this.tokens = {
      mint: (wid, mid, capabilities = [], opts = {}) =>
        this._req("POST", `/workspaces/${wid}/members/${mid}/tokens`, {
          capabilities,
          ...(opts.label === undefined ? {} : { label: opts.label }),
          ...(opts.capabilitySet === undefined ? {} : { capability_set: opts.capabilitySet }),
          ...(opts.expiresAt === undefined ? {} : { expires_at: opts.expiresAt }),
        }),
      list: (wid, mid) => this._req("GET", `/workspaces/${wid}/members/${mid}/tokens`),
    };
    this.channels = {
      list: (wid) => this._req("GET", `/workspaces/${wid}/channels`),
      create: (wid, name, priv = false) =>
        this._req("POST", `/workspaces/${wid}/channels`, { name, private: priv }),
      /** The channel's boot prefix, byte for byte as served, with its sha256. */
      boot: async (cid) => bootPrefix(await this._reqRaw("GET", `/channels/${cid}/boot`)),
    };
    this.threads = {
      list: (cid, query) => this._req("GET", `/channels/${cid}/threads${qs(query)}`),
      /** Every live thread in the channel, fetching `pageSize` (at most MAX_PAGE_SIZE) per request. */
      listAll: (cid, opts = {}) => this._threadsAll(cid, pageSize(opts.pageSize)),
      create: (cid, title) => this._req("POST", `/channels/${cid}/threads`, { title }),
      get: (id) => this._req("GET", `/threads/${id}`),
      context: (id, query) => this._req("GET", `/threads/${id}/context${qs(query)}`),
      transition: (id, body) => this._req("POST", `/threads/${id}`, body),
      setResult: (id, result) => this._req("PUT", `/threads/${id}/result`, { result }),
      getResult: (id) => this._req("GET", `/threads/${id}/result`),
    };
    this.messages = {
      list: (tid, query) => this._req("GET", `/threads/${tid}/messages${qs(query)}`),
      post: (tid, body) => this._req("POST", `/threads/${tid}/messages`, { body }),
    };
    this.artifacts = {
      upload: (bytes, kind) => this._reqRaw("POST", `/artifacts?kind=${kind}`, bytes),
      get: (sha) => this._reqRaw("GET", `/artifacts/${sha}`),
      meta: (sha) => this._req("GET", `/artifacts/${sha}/meta`),
    };
  }

  /** POST /channels/{cid}/threads/claim-next — readiness/skill/lease-aware. */
  claimNextThread(cid, body) {
    return this._req("POST", `/channels/${cid}/threads/claim-next`, body || {});
  }
  /** POST /threads/{id}/claim/renew — holder-only lease heartbeat. */
  renewClaim(id, claimLeaseId, leaseSecs = 300) {
    return this._req("POST", `/threads/${id}/claim/renew`, {
      claim_lease_id: claimLeaseId,
      lease_secs: leaseSecs,
    });
  }

  async *_threadsAll(cid, pageSize) {
    let cursor;
    for (;;) {
      const query = { limit: pageSize };
      if (cursor) query.cursor = cursor;
      const page = (await this.threads.list(cid, query)) || [];
      yield* page;
      if (page.length < pageSize) return;
      cursor = page[page.length - 1].id;
    }
  }

  async *_eventsAll(wid, query) {
    const limit = pageSize(query.limit);
    let after = query.after_id || 0;
    for (;;) {
      const page = (await this.workspaces.events(wid, { ...query, after_id: after, limit })) || [];
      for (const row of page) {
        const id = row && (row.id ?? row.log_id);
        if (typeof id === "number") after = Math.max(after, id);
        yield row;
      }
      if (page.length < limit) return;
    }
  }

  /**
   * Send with retries. A write carries one `Idempotency-Key` across all its
   * attempts, so a retry after a lost response gets the first answer back
   * instead of writing twice.
   */
  async _send(method, path, headers, body) {
    const h = { authorization: `Bearer ${this.token}`, ...headers };
    if (WRITE_METHODS.has(method)) h["idempotency-key"] = newIdempotencyKey();
    for (let attempt = 0; ; attempt++) {
      let resp;
      try {
        resp = await this._fetch(`${this.baseUrl}${path}`, { method, headers: h, body });
      } catch (err) {
        if (attempt >= this.maxRetries) throw err;
        await this._sleep(retryDelayMs(attempt));
        continue;
      }
      this._captureRoomLsn(resp);
      if (attempt < this.maxRetries && (await retryable(resp))) {
        await resp.arrayBuffer().catch(() => undefined);
        await this._sleep(retryDelayMs(attempt, resp.headers.get("retry-after")));
        continue;
      }
      return resp;
    }
  }

  async _req(method, path, body) {
    const headers = {};
    let payload;
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      payload = JSON.stringify(body);
    }
    const resp = await this._send(method, path, headers, payload);
    return this._handle(resp);
  }

  async _reqRaw(method, path, body) {
    const resp = await this._send(method, path, {}, body);
    if (method === "GET") {
      if (!resp.ok) await this._raise(resp);
      return new Uint8Array(await resp.arrayBuffer());
    }
    return this._handle(resp);
  }

  _captureRoomLsn(resp) {
    const parsed = parseRoomLsn(resp.headers.get("maidan-room-lsn"));
    if (parsed !== undefined) this.lastRoomLsn = parsed;
  }

  async _handle(resp) {
    if (!resp.ok) await this._raise(resp);
    if (resp.status === 204) return undefined;
    const text = await resp.text();
    return text ? JSON.parse(text) : undefined;
  }

  async _raise(resp) {
    let parsed;
    const text = await resp.text().catch(() => "");
    try {
      parsed = text ? JSON.parse(text) : undefined;
    } catch {
      parsed = text;
    }
    const err = problemError(resp.status, parsed);
    const ra = resp.headers.get("retry-after");
    if (ra && Number.isFinite(Number(ra))) err.retryAfter = Number(ra);
    throw err;
  }

  /**
   * Subscribe to the event stream over WebSocket. `filter` follows
   * contracts/ws-subscribe-filter.schema.json (set `workspace_id` to enable
   * replay). Returns a handle with `close()`. Control frames (subscribe_ack,
   * schema_version, replay_*) are skipped; each domain event is passed to
   * `onEvent`. Unknown `kind`s are still delivered (forward-compat).
   * @returns {Promise<{ close: () => void }>}
   */
  subscribe(filter, onEvent, onError, opts = {}) {
    if (!this._WebSocket) {
      return Promise.reject(
        new Error("No WebSocket available; pass options.WebSocket (e.g. the `ws` package on Node <22)"),
      );
    }
    const wsUrl = `${this.baseUrl.replace(/^http/, "ws")}/ws/subscribe`;
    const ws = new this._WebSocket(wsUrl);
    return new Promise((resolve, reject) => {
      let settled = false;
      ws.onopen = () => {
        const frame = { filter: filter || {}, token: this.token };
        if (opts.afterId) frame.after_id = opts.afterId;
        if (opts.consumerId) frame.consumer_id = opts.consumerId;
        ws.send(JSON.stringify(frame));
        settled = true;
        resolve({ close: () => ws.close() });
      };
      ws.onerror = (e) => {
        if (!settled) reject(e);
        else if (onError) onError(e);
      };
      ws.onmessage = (ev) => {
        let frame;
        try {
          frame = JSON.parse(typeof ev.data === "string" ? ev.data : ev.data.toString());
        } catch {
          return;
        }
        if (frame && frame.type === "cursor_too_old") {
          onEvent(frame);
          ws.close();
          return;
        }
        if (frame && frame.type) return; // subscribe_ack, schema_version, replay_*
        if (frame && typeof frame.kind === "string") onEvent(frame);
      };
    });
  }

  /**
   * HTTP backfill GET /workspaces/{id}/events then WS cutover.
   * A 409 must_refetch throws MaidanError.isCursorTooOld — never clamped.
   */
  async follow(spec, onEvent, onError) {
    const limit = pageSize(spec.pageLimit);
    let after = spec.afterId || 0;
    for (;;) {
      const query = { after_id: after, limit };
      if (spec.channelId) query.channel_id = spec.channelId;
      if (spec.threadId) query.thread_id = spec.threadId;
      if (spec.types && spec.types.length) query.types = spec.types.join(",");
      if (spec.consumerId) query.consumer_id = spec.consumerId;
      const page = (await this.workspaces.events(spec.workspaceId, query)) || [];
      if (!page.length) break;
      for (const row of page) {
        const id = row && (row.id ?? row.log_id);
        if (typeof id === "number") after = Math.max(after, id);
        onEvent(normalizeStored(row));
      }
      if (page.length < limit) break;
    }
    const filter = { workspace_id: spec.workspaceId };
    if (spec.channelId) filter.channel_id = spec.channelId;
    if (spec.threadId) filter.thread_id = spec.threadId;
    if (spec.types && spec.types.length) filter.kinds = spec.types;
    return this.subscribe(filter, onEvent, onError, {
      afterId: after,
      consumerId: spec.consumerId,
    });
  }

  /** Resolve with the first event whose `kind` matches, or null after `timeoutMs`. */
  _waitForKind(filter, kind, timeoutMs = 30000) {
    return new Promise((resolve, reject) => {
      let handle;
      const timer = setTimeout(() => {
        if (handle) handle.close();
        resolve(null);
      }, timeoutMs);
      this.subscribe(
        { ...filter, kinds: [kind] },
        (event) => {
          clearTimeout(timer);
          if (handle) handle.close();
          resolve(event);
        },
        (e) => {
          clearTimeout(timer);
          reject(e);
        },
      ).then((h) => {
        handle = h;
      }, reject);
    });
  }

  waitForResult(threadId, workspaceId, timeoutMs) {
    return this._waitForKind({ workspace_id: workspaceId, thread_id: threadId }, "thread_result_set", timeoutMs);
  }
  waitForMention(memberId, workspaceId, timeoutMs) {
    return this._waitForKind({ workspace_id: workspaceId, member_id: memberId }, "mention_recorded", timeoutMs);
  }
  waitForReady(workspaceId, channelId, timeoutMs) {
    const f = { workspace_id: workspaceId };
    if (channelId) f.channel_id = channelId;
    return this._waitForKind(f, "thread_ready", timeoutMs);
  }
}

async function retryable(resp) {
  if (RETRY_STATUSES.has(resp.status)) return true;
  if (resp.status !== 409) return false;
  try {
    const body = await resp.clone().json();
    return body && body.type === IN_FLIGHT_TYPE;
  } catch {
    return false;
  }
}

function normalizeStored(row) {
  if (!row || typeof row !== "object") return { raw: row };
  const out = row.payload && typeof row.payload === "object" ? { ...row.payload } : { ...row };
  const id = row.id ?? row.log_id;
  if (id !== undefined) out.log_id = id;
  for (const key of ["kind", "workspace_id", "channel_id", "thread_id"]) {
    if (out[key] === undefined && row[key] !== undefined) out[key] = row[key];
  }
  return out;
}

function qs(query) {
  if (!query) return "";
  const s = new URLSearchParams(query).toString();
  return s ? `?${s}` : "";
}

export { UsageError, USAGE_PROVIDERS, normalizeUsage, usdMicros } from "./usage.js";

export default Client;
