// The live connection's rules, driven by a fake socket and a manual clock.
import { test } from "node:test";
import assert from "node:assert/strict";
import { LIVE_POLL_MS, LiveConnection, retryDelay, type Clock, type SocketLike } from "../src/core/live.ts";

class FakeSocket implements SocketLike {
  readyState = 0;
  sent: string[] = [];
  closed = false;
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onclose: ((ev: { code?: number; reason?: string }) => void) | null = null;
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.closed = true;
  }
  open() {
    this.readyState = 1;
    this.onopen?.({});
  }
  frame(v: unknown) {
    this.onmessage?.({ data: JSON.stringify(v) });
  }
  drop(code = 1006, reason = "") {
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }
}

function harness(stored: string | null = null) {
  let now = 0;
  let nextId = 1;
  const timers = new Map<number, { at: number; fn: () => void; every?: number }>();
  const clock: Clock = {
    setTimeout: (fn, ms) => (timers.set(nextId, { at: now + ms, fn }), nextId++),
    clearTimeout: (id) => void timers.delete(id as number),
    setInterval: (fn, ms) => (timers.set(nextId, { at: now + ms, fn, every: ms }), nextId++),
    clearInterval: (id) => void timers.delete(id as number),
    random: () => 0.5,
  };
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      const due = [...timers.entries()].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      const [id, t] = due;
      now = t.at;
      if (t.every) t.at += t.every;
      else timers.delete(id);
      t.fn();
    }
    now = end;
  };
  const store = new Map<string, string>();
  if (stored) store.set("resume", stored);
  const storage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
  };
  const sockets: FakeSocket[] = [];
  const events: number[] = [];
  let polls = 0;
  let recovered = 0;
  const conn = new LiveConnection({
    url: "ws://x/ws/subscribe",
    openSocket: () => {
      const s = new FakeSocket();
      sockets.push(s);
      return s;
    },
    storage,
    resumeKey: "resume",
    clock,
    token: () => "",
    memberId: () => "mem_1",
    onEvent: (f) => events.push(f.log_id),
    onPoll: () => polls++,
    onRecovered: () => recovered++,
  });
  return { conn, sockets, events, store, advance, polls: () => polls, recovered: () => recovered };
}

test("backoff doubles from 1.5 s to a 30 s cap", () => {
  assert.equal(retryDelay(0, 0.5), 1500);
  assert.equal(retryDelay(1, 0.5), 3000);
  assert.equal(retryDelay(10, 0.5), 30000);
});

test("the first subscribe starts at the head; the ack's resume token is stored", () => {
  const h = harness();
  h.conn.connect({ workspace_id: "ws_1" });
  h.sockets[0].open();
  assert.deepEqual(JSON.parse(h.sockets[0].sent[0]), { filter: { workspace_id: "ws_1" }, after_id: 0, member_id: "mem_1" });
  h.sockets[0].frame({ type: "subscribe_ack", after_id: 7, resume_token: "rt_1" });
  assert.equal(h.conn.getStatus().kind, "connected");
  assert.equal(h.store.get("resume"), "rt_1");
  h.sockets[0].frame({ log_id: 9, kind: "message_posted" });
  assert.deepEqual(h.events, [9]);
  assert.equal(h.conn.afterId, 9);
});

test("a drop polls while down, reconnects with the resume token, and reconciles once back", () => {
  const h = harness("rt_saved");
  h.conn.connect({ workspace_id: "ws_1" });
  h.sockets[0].open();
  h.sockets[0].frame({ type: "subscribe_ack", after_id: 3 });
  h.sockets[0].drop();
  assert.equal(h.conn.getStatus().kind, "reconnecting");
  assert.ok(h.conn.polling);
  h.advance(1500);
  assert.equal(h.sockets.length, 2);
  h.sockets[1].open();
  assert.equal(JSON.parse(h.sockets[1].sent[0]).resume_token, "rt_saved");
  h.sockets[1].drop();
  h.advance(LIVE_POLL_MS);
  assert.ok(h.polls() >= 1);
  h.advance(3000);
  h.sockets[h.sockets.length - 1].open();
  h.sockets[h.sockets.length - 1].frame({ type: "subscribe_ack", after_id: 3 });
  assert.equal(h.conn.getStatus().kind, "connected");
  assert.equal(h.conn.polling, false);
  assert.equal(h.recovered(), 1);
});

test("a policy close stops: no retry and no polling", () => {
  const h = harness();
  h.conn.connect({ workspace_id: "ws_1" });
  h.sockets[0].open();
  h.sockets[0].drop(1008, "missing event:subscribe");
  assert.deepEqual(h.conn.getStatus(), { kind: "refused", reason: "missing event:subscribe" });
  h.advance(60000);
  assert.equal(h.sockets.length, 1);
  assert.equal(h.conn.polling, false);
});

test("a replaced socket's late frames and close are ignored", () => {
  const h = harness();
  h.conn.connect({ workspace_id: "ws_1" });
  const old = h.sockets[0];
  h.conn.connect({ workspace_id: "ws_1", channel_id: "ch_1" });
  assert.ok(old.closed);
  old.frame({ log_id: 99 });
  old.drop();
  assert.deepEqual(h.events, []);
  assert.equal(h.conn.getStatus().kind, "connecting");
  assert.equal(h.conn.polling, false);
});

test("disconnect stops retrying; forgetResume drops the stored token", () => {
  const h = harness("rt_saved");
  h.conn.connect({ workspace_id: "ws_1" });
  h.conn.disconnect();
  h.advance(60000);
  assert.equal(h.sockets.length, 1);
  assert.equal(h.conn.getStatus().kind, "idle");
  h.conn.forgetResume();
  assert.equal(h.store.has("resume"), false);
});
