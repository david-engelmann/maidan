// Maidan TypeScript client (v1 surface). REST + WebSocket, dependency-free:
// uses the global `fetch` (Node 18+) and a pluggable WebSocket (global in the
// browser / Node 22+, or inject one via `options.WebSocket`). See docs/Client
// Contract.md for the frozen surface.

export class MaidanError extends Error {
  constructor(status, body, message) {
    super(message || `Maidan request failed: HTTP ${status}`);
    this.name = "MaidanError";
    this.status = status;
    this.body = body;
    // Retry-After (seconds) surfaced on 429 (server rate limit, Cluster 172).
    this.retryAfter = undefined;
  }
  get isConflict() {
    return this.status === 409;
  }
  get isForbidden() {
    return this.status === 403;
  }
  get isRateLimited() {
    return this.status === 429;
  }
}

function envDefault(key) {
  return typeof process !== "undefined" && process.env ? process.env[key] : undefined;
}

export class Client {
  /**
   * @param {string} [baseUrl] defaults to MAIDAN_URL
   * @param {string} [token] defaults to MAIDAN_TOKEN
   * @param {{ fetch?: typeof fetch, WebSocket?: any }} [options]
   */
  constructor(baseUrl, token, options = {}) {
    this.baseUrl = (baseUrl || envDefault("MAIDAN_URL") || "http://127.0.0.1:8080").replace(
      /\/+$/,
      "",
    );
    this.token = token || envDefault("MAIDAN_TOKEN") || "";
    this._fetch = options.fetch || (typeof fetch !== "undefined" ? fetch : undefined);
    this._WebSocket = options.WebSocket || (typeof WebSocket !== "undefined" ? WebSocket : undefined);

    // MCP is a URL, not a dependency (docs/Client Contract.md §4).
    this.mcpUrl = `${this.baseUrl}/mcp/streamable`;

    this.workspaces = {
      create: (name) => this._req("POST", "/workspaces", { name }),
      get: (id) => this._req("GET", `/workspaces/${id}`),
      import: (bundle, mode) =>
        this._req("POST", `/workspaces/import${mode ? `?mode=${mode}` : ""}`, bundle),
    };
    this.channels = {
      list: (wid) => this._req("GET", `/workspaces/${wid}/channels`),
      create: (wid, name, priv = false) =>
        this._req("POST", `/workspaces/${wid}/channels`, { name, private: priv }),
    };
    this.threads = {
      create: (cid, title) => this._req("POST", `/channels/${cid}/threads`, { title }),
      get: (id) => this._req("GET", `/threads/${id}`),
      context: (id, query) => this._req("GET", `/threads/${id}/context${qs(query)}`),
      transition: (id, body) => this._req("POST", `/threads/${id}`, body),
      setResult: (id, result) => this._req("PUT", `/threads/${id}/result`, { result }),
      getResult: (id) => this._req("GET", `/threads/${id}/result`),
    };
    this.messages = {
      list: (tid, query) => this._req("GET", `/threads/${tid}/messages${qs(query)}`),
      post: (tid, authorId, body) =>
        this._req("POST", `/threads/${tid}/messages`, { author_id: authorId, body }),
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
  renewClaim(id) {
    return this._req("POST", `/threads/${id}/claim/renew`, {});
  }

  async _req(method, path, body) {
    const headers = { authorization: `Bearer ${this.token}` };
    const init = { method, headers };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    const resp = await this._fetch(`${this.baseUrl}${path}`, init);
    return this._handle(resp);
  }

  async _reqRaw(method, path, body) {
    const headers = { authorization: `Bearer ${this.token}` };
    const init = { method, headers };
    if (body !== undefined) init.body = body;
    const resp = await this._fetch(`${this.baseUrl}${path}`, init);
    if (method === "GET") {
      if (!resp.ok) await this._raise(resp);
      return new Uint8Array(await resp.arrayBuffer());
    }
    return this._handle(resp);
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
    const err = new MaidanError(resp.status, parsed);
    if (resp.status === 429) {
      const ra = resp.headers.get("retry-after");
      if (ra) err.retryAfter = Number(ra);
    }
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
  subscribe(filter, onEvent, onError) {
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
        ws.send(JSON.stringify({ filter: filter || {}, token: this.token }));
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
        if (frame && frame.type) return; // control frame (subscribe_ack, replay_hint, …)
        if (frame && typeof frame.kind === "string") onEvent(frame);
      };
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

function qs(query) {
  if (!query) return "";
  const s = new URLSearchParams(query).toString();
  return s ? `?${s}` : "";
}

export default Client;
