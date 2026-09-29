// Unit tests for retries, idempotency keys and auto-paging, over a fake fetch
// (no server needed).
import { test } from "node:test";
import assert from "node:assert/strict";
import { Client, MaidanError, retryDelayMs } from "./index.js";

function fakeFetch(answers) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, init });
    const next = answers.shift();
    if (next instanceof Error) throw next;
    const [status, body, headers = {}] = next;
    return new Response(body === undefined ? null : JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json", ...headers },
    });
  };
  return { fetch, calls };
}

function client(answers, opts = {}) {
  const f = fakeFetch(answers);
  const sleeps = [];
  const c = new Client("http://x", "t", {
    fetch: f.fetch,
    sleep: async (ms) => { sleeps.push(ms); },
    ...opts,
  });
  return { c, calls: f.calls, sleeps };
}

test("a write retries a lost response with the same Idempotency-Key", async () => {
  const { c, calls } = client([new TypeError("socket hang up"), [201, { id: "m1" }]]);
  const msg = await c.messages.post("t1", "hi");
  assert.equal(msg.id, "m1");
  assert.equal(calls.length, 2);
  const k0 = calls[0].init.headers["idempotency-key"];
  assert.ok(k0 && k0.length >= 16);
  assert.equal(calls[1].init.headers["idempotency-key"], k0);
});

test("each logical write gets its own key; reads get none", async () => {
  const { c, calls } = client([[201, {}], [201, {}], [200, []]]);
  await c.messages.post("t1", "a");
  await c.messages.post("t1", "b");
  await c.channels.list("w");
  assert.notEqual(
    calls[0].init.headers["idempotency-key"],
    calls[1].init.headers["idempotency-key"],
  );
  assert.equal(calls[2].init.headers["idempotency-key"], undefined);
});

test("429 honours Retry-After; 5xx backs off; the retry budget is bounded", async () => {
  const { c, calls, sleeps } = client([
    [429, { type: "x" }, { "retry-after": "3" }],
    [503, { type: "x" }],
    [503, { type: "x", detail: "still down" }],
  ]);
  await assert.rejects(() => c.channels.list("w"), (err) => {
    assert.ok(err instanceof MaidanError);
    assert.equal(err.status, 503);
    return true;
  });
  assert.equal(calls.length, 3, "1 try + maxRetries (2)");
  assert.equal(sleeps[0], 3000);
  assert.ok(sleeps[1] >= 500 && sleeps[1] <= 1000, `${sleeps[1]}`);
});

test("409 in flight for the key is retried; a plain 409 is not", async () => {
  const inFlight = { type: "https://maidan.dev/problems/idempotency-key-in-flight" };
  const { c, calls } = client([[409, inFlight], [201, { id: "c" }]]);
  assert.equal((await c.channels.create("w", "n")).id, "c");
  assert.equal(calls.length, 2);

  const plain = client([[409, { type: "https://maidan.dev/problems/conflict" }]]);
  await assert.rejects(() => plain.c.channels.create("w", "n"), (e) => e.status === 409);
  assert.equal(plain.calls.length, 1);
});

test("4xx other than 408/409-in-flight/429 is not retried; maxRetries 0 disables", async () => {
  const { c, calls } = client([[403, { type: "forbidden" }]]);
  await assert.rejects(() => c.channels.list("w"), (e) => e.status === 403);
  assert.equal(calls.length, 1);
  const off = client([[503, {}]], { maxRetries: 0 });
  await assert.rejects(() => off.c.channels.list("w"), (e) => e.status === 503);
  assert.equal(off.calls.length, 1);
});

test("retryDelayMs: exponential with jitter, capped", () => {
  assert.equal(retryDelayMs(0, null, () => 0), 250);
  assert.equal(retryDelayMs(0, null, () => 1), 500);
  assert.equal(retryDelayMs(10, null, () => 1), 8000);
  assert.equal(retryDelayMs(0, "120"), 60000);
});

test("threads.listAll pages by cursor until a short page", async () => {
  const t = (id) => ({ id });
  const { c, calls } = client([
    [200, [t("a"), t("b")]],
    [200, [t("c"), t("d")]],
    [200, [t("e")]],
  ]);
  const ids = [];
  for await (const th of c.threads.listAll("ch", { pageSize: 2 })) ids.push(th.id);
  assert.deepEqual(ids, ["a", "b", "c", "d", "e"]);
  assert.match(calls[0].url, /\/channels\/ch\/threads\?limit=2$/);
  assert.match(calls[1].url, /limit=2&cursor=b$/);
  assert.match(calls[2].url, /limit=2&cursor=d$/);
});

test("workspaces.eventsAll pages by after_id", async () => {
  const { c, calls } = client([
    [200, [{ id: 1 }, { id: 2 }]],
    [200, [{ id: 3 }]],
  ]);
  const ids = [];
  for await (const ev of c.workspaces.eventsAll("w", { limit: 2 })) ids.push(ev.id);
  assert.deepEqual(ids, [1, 2, 3]);
  assert.match(calls[1].url, /after_id=2/);
});
