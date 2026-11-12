import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { RETRY_KEY_MS, ReachStore, bumpAuthEpoch, forgetRetryKeys, guarded, offlineBanner, reach, writeInit } from "../src/core/api.ts";

// UI-008: a person's retry of a write whose answer never came is the same
// write (the same Idempotency-Key, so the server replays the first result),
// and while the server is out of reach every place says its data may be old.

type Step = number | "drop";
function server(steps: Step[]) {
  const keys: (string | undefined)[] = [];
  const fetchFn = async (_url: string, init?: RequestInit): Promise<Response> => {
    keys.push((init?.headers as Record<string, string> | undefined)?.["Idempotency-Key"]);
    const step = steps.shift() ?? 200;
    if (step === "drop") throw new TypeError("Failed to fetch");
    return new Response("{}", { status: step });
  };
  return { keys, send: guarded(fetchFn) };
}
const post = (body: string) => writeInit({ method: "POST", headers: { "Content-Type": "application/json" }, body });

beforeEach(() => forgetRetryKeys());

test("a retry after a dropped connection reuses the key; after a settled answer the same request is new", async () => {
  const { keys, send } = server(["drop", 201, 201]);
  await assert.rejects(send("/threads/t1/messages", post('{"body":"ship it"}')));
  await send("/threads/t1/messages", post('{"body":"ship it"}'));
  await send("/threads/t1/messages", post('{"body":"ship it"}'));
  assert.equal(keys[0], keys[1], "the retry is the same write");
  assert.notEqual(keys[1], keys[2], "the same words sent again after a success are a new message");
});

test("a 5xx, 408 or 429 leaves the outcome open; a 4xx settles it", async () => {
  for (const open of [500, 502, 503, 408, 429]) {
    forgetRetryKeys();
    const { keys, send } = server([open, 200]);
    await send("/x", post("{}"));
    await send("/x", post("{}"));
    assert.equal(keys[0], keys[1], `${open} keeps the key`);
  }
  const { keys, send } = server([400, 200]);
  await send("/x", post("{}"));
  await send("/x", post("{}"));
  assert.notEqual(keys[0], keys[1], "a refused write was not made, so a new try is a new key");
});

test("only the identical request reuses a key: another URL, body or method is its own write", async () => {
  const { keys, send } = server(["drop", "drop", "drop", "drop"]);
  await assert.rejects(send("/threads/t1/messages", post('{"body":"a"}')));
  await assert.rejects(send("/threads/t2/messages", post('{"body":"a"}')));
  await assert.rejects(send("/threads/t1/messages", post('{"body":"b"}')));
  await assert.rejects(send("/threads/t1/messages", writeInit({ method: "PUT", headers: {}, body: '{"body":"a"}' })));
  assert.equal(new Set(keys).size, 4);
});

test("a kept key expires, and a new credential starts fresh", async () => {
  const realNow = Date.now;
  try {
    let now = 1_000_000;
    Date.now = () => now;
    const a = server(["drop", 200]);
    await assert.rejects(a.send("/x", post("{}")));
    now += RETRY_KEY_MS + 1;
    await a.send("/x", post("{}"));
    assert.notEqual(a.keys[0], a.keys[1]);
  } finally {
    Date.now = realNow;
  }
  const b = server(["drop", 200]);
  await assert.rejects(b.send("/y", post("{}")));
  bumpAuthEpoch();
  await b.send("/y", post("{}"));
  assert.notEqual(b.keys[0], b.keys[1]);
});

// A server whose answers the test hands out one at a time, so requests overlap.
function heldServer() {
  const keys: string[] = [];
  const waiting: ((step: Step) => void)[] = [];
  const fetchFn = (_url: string, init?: RequestInit): Promise<Response> => {
    keys.push((init?.headers as Record<string, string>)["Idempotency-Key"]);
    return new Promise((resolve, reject) => {
      waiting.push((step) => (step === "drop" ? reject(new TypeError("Failed to fetch")) : resolve(new Response("{}", { status: step }))));
    });
  };
  return { keys, send: guarded(fetchFn), answer: (i: number, step: Step) => waiting[i](step) };
}

test("an identical write sent while the first is still pending gets its own key", async () => {
  const { keys, send, answer } = heldServer();
  const first = send("/threads/t1/messages", post('{"body":"ship it"}'));
  const second = send("/threads/t1/messages", post('{"body":"ship it"}'));
  assert.notEqual(keys[0], keys[1], "a pending write's key is not shared, so the server never answers the second with 409 in flight");
  answer(0, 201);
  answer(1, 201);
  assert.equal((await first).status, 201);
  assert.equal((await second).status, 201);
  // A retry that is itself pending holds the kept key the same way.
  const third = send("/x", post("{}"));
  answer(2, "drop");
  await assert.rejects(third);
  const retry = send("/x", post("{}"));
  const other = send("/x", post("{}"));
  assert.equal(keys[3], keys[2], "the retry reuses the dropped write's key");
  assert.notEqual(keys[4], keys[2], "a write that starts while the retry is pending does not");
  answer(3, 200);
  answer(4, 200);
  await Promise.all([retry, other]);
});

test("an answer under an older credential leaves the newer request's kept key alone", async () => {
  const { keys, send, answer } = heldServer();
  const old = send("/y", post("{}"));
  bumpAuthEpoch();
  const fresh = send("/y", post("{}"));
  answer(1, "drop");
  await assert.rejects(fresh);
  answer(0, 200);
  await old;
  const retry = send("/y", post("{}"));
  answer(2, 200);
  await retry;
  assert.equal(keys[2], keys[1], "the old request's success did not settle the new credential's write");
  // And an old request whose answer never came does not seed the new credential's key.
  const stale = send("/z", post("{}"));
  bumpAuthEpoch();
  answer(3, "drop");
  await assert.rejects(stale);
  const next = send("/z", post("{}"));
  answer(4, 200);
  await next;
  assert.notEqual(keys[4], keys[3]);
});

test("a read carries no key and is not tracked", async () => {
  const { keys, send } = server([200]);
  await send("/x", { method: "GET" });
  assert.deepEqual(keys, [undefined]);
});

test("a dropped request marks the server unreachable; any answer marks it back", async () => {
  const { send } = server(["drop", 404]);
  await assert.rejects(send("/x", { method: "GET" }));
  assert.match(offlineBanner(reach.getSnapshot()) ?? "", /^Can't reach the server\. What you see may be out of date/);
  await send("/x", { method: "GET" });
  assert.equal(offlineBanner(reach.getSnapshot()), null);
});

test("the banner: offline in the browser first, and since when once it is old", () => {
  const r = new ReachStore();
  let told = 0;
  r.subscribe(() => told++);
  assert.equal(offlineBanner(r.getSnapshot()), null);
  r.answered(Date.parse("2026-11-11T15:42:00Z"));
  r.answered(Date.parse("2026-11-11T15:43:00Z"));
  assert.equal(told, 0, "answers while reachable change nothing on screen");
  r.browser(false);
  assert.equal(told, 1);
  const now = Date.parse("2026-11-11T15:43:30Z");
  assert.match(offlineBanner(r.getSnapshot(), now) ?? "", /^You're offline\. What you see may be out of date, and changes won't send until the connection is back\.$/);
  assert.match(offlineBanner(r.getSnapshot(), now + 10 * 60_000) ?? "", /^You're offline\. What you see is from .+ and may be out of date/);
  r.browser(true);
  assert.equal(offlineBanner(r.getSnapshot()), null);
  // A request that failed while offline does not hold the line up once the browser is back.
  r.browser(false);
  r.unreachable();
  r.browser(true);
  assert.equal(offlineBanner(r.getSnapshot()), null);
});
