// The live connection: one WebSocket to /ws/subscribe, resumed after a drop,
// with the board polled while it is down. A port of the board's realtime.js
// with every timer, socket and store injected, so the rules are testable:
//
// - A drop retries with backoff, 1.5 s doubling to 30 s, with jitter.
// - While the socket is down, `onPoll` runs every LIVE_POLL_MS.
// - The resume token from the ack is kept in storage; a reconnect sends it
//   (or the last log id when there is none), so only missed events replay.
// - A policy close (1008: bad token, missing capability) will not fix itself:
//   it stops, says why, and does not poll.
// - A socket replaced by a newer one is ignored: its late events and close
//   must not touch the new one.

export const LIVE_POLL_MS = 15000;
export const RETRY_BASE_MS = 1500;
export const RETRY_MAX_MS = 30000;

export type LiveFilter = {
  workspace_id: string;
  channel_id?: string;
  thread_id?: string;
  kinds?: string[];
};

export type LiveStatus =
  | { kind: "idle" }
  | { kind: "connecting" }
  | { kind: "connected" }
  | { kind: "reconnecting"; inMs: number }
  | { kind: "refused"; reason: string }
  | { kind: "closed" };

export type EventFrame = { log_id: number; kind?: string; [key: string]: unknown };
export type ControlFrame = { type: string; [key: string]: unknown };

export type SocketLike = {
  readonly readyState: number;
  send(data: string): void;
  close(): void;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null;
};

export type StorageLike = Pick<Storage, "getItem" | "setItem" | "removeItem">;

export type Clock = {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(id: unknown): void;
  setInterval(fn: () => void, ms: number): unknown;
  clearInterval(id: unknown): void;
  random(): number;
};

export type LiveOptions = {
  url: string;
  openSocket: (url: string) => SocketLike;
  storage: StorageLike;
  resumeKey: string;
  clock?: Clock;
  /** A token pasted in this tab only; a session cookie needs none. */
  token?: () => string;
  /** The member presence and typing go to. */
  memberId?: () => string | null;
  onEvent?: (frame: EventFrame) => void;
  onControl?: (frame: ControlFrame) => void;
  /** Runs every LIVE_POLL_MS while the socket is down. */
  onPoll?: () => void;
  /** Runs once when the socket is back after a drop. */
  onRecovered?: () => void;
};

const OPEN = 1;

const realClock: Clock = {
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (id) => globalThis.clearTimeout(id as ReturnType<typeof setTimeout>),
  setInterval: (fn, ms) => globalThis.setInterval(fn, ms),
  clearInterval: (id) => globalThis.clearInterval(id as ReturnType<typeof setInterval>),
  random: () => Math.random(),
};

export function retryDelay(retries: number, random: number): number {
  return Math.min(RETRY_MAX_MS, RETRY_BASE_MS * 2 ** retries) * (0.8 + random * 0.4);
}

export class LiveConnection {
  private socket: SocketLike | null = null;
  private wanted = false;
  private retries = 0;
  private retryTimer: unknown = null;
  private pollTimer: unknown = null;
  private filter: LiveFilter | null = null;
  private status: LiveStatus = { kind: "idle" };
  private listeners = new Set<() => void>();
  private opts: LiveOptions;
  private readonly clock: Clock;
  afterId = 0;
  resumeToken: string | null;

  constructor(opts: LiveOptions) {
    this.opts = opts;
    this.clock = opts.clock ?? realClock;
    this.resumeToken = opts.storage.getItem(opts.resumeKey);
  }

  /** Swap the callbacks and credential readers (each render's latest). */
  update(patch: Partial<Pick<LiveOptions, "token" | "memberId" | "onEvent" | "onControl" | "onPoll" | "onRecovered">>): void {
    this.opts = { ...this.opts, ...patch };
  }

  getStatus = (): LiveStatus => this.status;

  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  get polling(): boolean {
    return this.pollTimer !== null;
  }

  private setStatus(status: LiveStatus): void {
    this.status = status;
    this.listeners.forEach((fn) => fn());
  }

  private setPolling(on: boolean): void {
    if (on && this.pollTimer === null) {
      this.pollTimer = this.clock.setInterval(() => this.opts.onPoll?.(), LIVE_POLL_MS);
    } else if (!on && this.pollTimer !== null) {
      this.clock.clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  connect(filter: LiveFilter): void {
    this.filter = filter;
    // Wanted from the first attempt, not from the ack: a drop before the ack
    // is retried like any other.
    this.wanted = true;
    this.open();
  }

  /** The user asked to stop: no retry, no polling. */
  disconnect(): void {
    this.wanted = false;
    this.clock.clearTimeout(this.retryTimer);
    this.setPolling(false);
    this.drop();
    this.setStatus({ kind: "idle" });
  }

  /** Back online or back in the foreground: skip the wait. */
  reconnectNow(): void {
    if (this.wanted && !this.socket) {
      this.clock.clearTimeout(this.retryTimer);
      this.open();
    }
  }

  /** Forget the resume point (sign-out, workspace switch). */
  forgetResume(): void {
    this.resumeToken = null;
    this.afterId = 0;
    this.opts.storage.removeItem(this.opts.resumeKey);
  }

  sendPresence(status: "online" | "away" | "offline"): boolean {
    if (!this.socket || this.socket.readyState !== OPEN) return false;
    this.socket.send(JSON.stringify({ type: "presence", status }));
    return true;
  }

  private drop(): void {
    if (this.socket) {
      const old = this.socket;
      this.socket = null;
      old.close();
    }
  }

  private subscribeFrame(): Record<string, unknown> {
    const frame: Record<string, unknown> = { filter: this.filter };
    const token = this.opts.token?.().trim();
    if (token) frame.token = token;
    if (this.resumeToken) frame.resume_token = this.resumeToken;
    else frame.after_id = this.afterId;
    const member = this.opts.memberId?.();
    if (member) frame.member_id = member;
    return frame;
  }

  private open(): void {
    this.clock.clearTimeout(this.retryTimer);
    this.drop();
    if (!this.filter) return;
    if (!this.retries) this.setStatus({ kind: "connecting" });
    const sock = this.opts.openSocket(this.opts.url);
    this.socket = sock;
    let acked = false;
    sock.onopen = () => {
      if (sock === this.socket) sock.send(JSON.stringify(this.subscribeFrame()));
    };
    sock.onmessage = (ev) => {
      if (sock !== this.socket) return;
      let v: unknown;
      try {
        v = JSON.parse(String(ev.data));
      } catch {
        return;
      }
      if (!v || typeof v !== "object") return;
      const frame = v as Record<string, unknown>;
      if (frame.type === "subscribe_ack") {
        acked = true;
        const recovered = this.retries > 0;
        this.retries = 0;
        this.setPolling(false);
        if (typeof frame.after_id === "number") this.afterId = frame.after_id;
        if (typeof frame.resume_token === "string" && frame.resume_token) {
          this.resumeToken = frame.resume_token;
          this.opts.storage.setItem(this.opts.resumeKey, frame.resume_token);
        }
        this.setStatus({ kind: "connected" });
        // Back after a drop: reconcile what the replay may not cover.
        if (recovered) this.opts.onRecovered?.();
        this.opts.onControl?.(frame as ControlFrame);
        return;
      }
      if (typeof frame.log_id === "number") {
        this.afterId = Math.max(this.afterId, frame.log_id);
        this.opts.onEvent?.(frame as EventFrame);
        return;
      }
      if (typeof frame.type === "string") this.opts.onControl?.(frame as ControlFrame);
    };
    sock.onclose = (ev) => {
      if (sock !== this.socket) return;
      this.socket = null;
      if (ev?.code === 1008) {
        // A refusal will not fix itself: say why and stop.
        this.wanted = false;
        this.setPolling(false);
        this.setStatus({ kind: "refused", reason: ev.reason || "the server refused the connection" });
        return;
      }
      if (!this.wanted) {
        this.setStatus(acked ? { kind: "closed" } : { kind: "idle" });
        return;
      }
      this.scheduleReconnect();
    };
    sock.onerror = () => {
      /* onclose follows and decides */
    };
  }

  private scheduleReconnect(): void {
    this.clock.clearTimeout(this.retryTimer);
    const delay = retryDelay(this.retries, this.clock.random());
    this.retries += 1;
    this.setPolling(true);
    this.setStatus({ kind: "reconnecting", inMs: delay });
    this.retryTimer = this.clock.setTimeout(() => this.open(), delay);
  }
}
