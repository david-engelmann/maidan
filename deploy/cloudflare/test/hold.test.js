import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { containerUnavailableKind } from "../src/edge.js";
import {
  FIRST_WAIT_MS,
  HOLD_MS,
  LONGEST_WAIT_MS,
  maySendAgain,
  REPLAY_BODY_LIMIT,
  replayableBody,
  sendHeld,
} from "../src/hold.js";

const plain = (status, body) => new Response(body, { status, headers: { "content-type": "text/plain;charset=UTF-8" } });
const ok = (body = "ok") => new Response(body, { status: 200, headers: { "content-type": "application/json" } });
const notRunning = () => plain(500, "Error proxying request to container: The container is not running, consider calling start()");
const lost = () => plain(500, "Container suddenly disconnected, try again");

// A fake clock: `wait` advances it, and each container call costs `callMs`.
function clock(callMs = 0) {
  let t = 0;
  return {
    now: () => t,
    wait: async (ms) => {
      t += ms;
    },
    spend: () => {
      t += callMs;
    },
    get t() {
      return t;
    },
  };
}

// A container that is gone until `upAt` (ms on the clock), then answers.
function replacedContainer(c, upAt, down = notRunning) {
  const calls = { send: 0, probe: 0 };
  const answer = (what) => async () => {
    calls[what] += 1;
    c.spend();
    return c.t >= upAt ? ok(what) : down();
  };
  return { calls, send: answer("send"), probe: answer("probe") };
}

test("the runtime's answers without a server are told apart from the server's", async () => {
  assert.equal(await containerUnavailableKind(notRunning()), "unreached");
  assert.equal(await containerUnavailableKind(plain(500, "Failed to start container: boom")), "unreached");
  assert.equal(await containerUnavailableKind(plain(503, "There is no Container instance available at this time.")), "unreached");
  assert.equal(await containerUnavailableKind(plain(500, "Error proxying request to container: Network error")), "unknown");
  assert.equal(await containerUnavailableKind(lost()), "unknown");
  assert.equal(await containerUnavailableKind(ok()), null);
  assert.equal(await containerUnavailableKind(plain(500, "something else")), null);
  const problem = new Response("{}", { status: 503, headers: { "content-type": "application/problem+json" } });
  assert.equal(await containerUnavailableKind(problem), null);
});

test("a read is held through a replacement and answered by the new server", async () => {
  const c = clock(100);
  const box = replacedContainer(c, 30_000);
  const res = await sendHeld({ method: "GET", body: null, ...box, now: c.now, wait: c.wait });
  assert.equal(res.status, 200);
  assert.equal(await res.text(), "send");
  assert.ok(c.t >= 30_000 && c.t < 30_000 + LONGEST_WAIT_MS + 1_000, `answered at ${c.t}`);
  // The request itself is sent once more only after /health answered.
  assert.equal(box.calls.send, 2);
});

test("the probes back off and never wait longer than the cap", async () => {
  const waits = [];
  const c = clock();
  const box = replacedContainer(c, 20_000);
  await sendHeld({
    method: "GET",
    body: null,
    ...box,
    now: c.now,
    wait: async (ms) => {
      waits.push(ms);
      await c.wait(ms);
    },
  });
  assert.equal(waits[0], FIRST_WAIT_MS);
  assert.ok(waits.every((ms) => ms <= LONGEST_WAIT_MS), waits.join(","));
  assert.ok(waits.slice(0, 3).every((ms, i) => i === 0 || ms > waits[i - 1]), waits.join(","));
});

test("past the hold the caller gets nothing and sends its 503", async () => {
  const c = clock(100);
  const box = replacedContainer(c, Infinity);
  assert.equal(await sendHeld({ method: "GET", body: null, ...box, now: c.now, wait: c.wait }), null);
  assert.ok(c.t >= HOLD_MS && c.t < HOLD_MS + 1_000, `gave up at ${c.t}`);
  assert.equal(box.calls.send, 1);
});

test("a write that never reached a server is sent again with its body", async () => {
  const c = clock();
  const box = replacedContainer(c, 5_000);
  const res = await sendHeld({ method: "POST", body: new ArrayBuffer(3), ...box, now: c.now, wait: c.wait });
  assert.equal(res.status, 200);
  assert.equal(box.calls.send, 2);
});

test("a write that may have reached a server is not sent twice", async () => {
  const c = clock();
  const box = replacedContainer(c, 5_000, lost);
  assert.equal(await sendHeld({ method: "POST", body: new ArrayBuffer(3), ...box, now: c.now, wait: c.wait }), null);
  assert.equal(box.calls.send, 1);
  assert.equal(box.calls.probe, 0);
  // A read may be, whatever happened to it.
  const read = replacedContainer(c, c.t + 5_000, lost);
  const res = await sendHeld({ method: "GET", body: null, ...read, now: c.now, wait: c.wait });
  assert.equal(res.status, 200);
});

test("a body too large to copy is sent once", async () => {
  const c = clock();
  const box = replacedContainer(c, 5_000);
  assert.equal(await sendHeld({ method: "POST", body: undefined, ...box, now: c.now, wait: c.wait }), null);
  assert.equal(box.calls.send, 1);
  assert.equal(maySendAgain("GET", "unknown", null), true);
  assert.equal(maySendAgain("POST", "unreached", undefined), false);
});

test("a thrown fetch (a Durable Object reset by the deploy) holds a read too", async () => {
  const c = clock();
  let thrown = 0;
  const res = await sendHeld({
    method: "GET",
    body: null,
    send: async () => {
      if (thrown++ === 0) throw new Error("Durable Object reset because its code was updated.");
      return ok();
    },
    probe: async () => ok(),
    now: c.now,
    wait: c.wait,
  });
  assert.equal(res.status, 200);
});

test("an answer from the server, even an error, is returned at once", async () => {
  const c = clock();
  let probes = 0;
  const problem = new Response("{}", { status: 500, headers: { "content-type": "application/problem+json" } });
  const res = await sendHeld({
    method: "GET",
    body: null,
    send: async () => problem,
    probe: async () => {
      probes += 1;
      return ok();
    },
    now: c.now,
    wait: c.wait,
  });
  assert.equal(res, problem);
  assert.equal(probes, 0);
  assert.equal(c.t, 0);
});

test("only a body of known, small length is copied", async () => {
  const post = (body, headers = {}) => new Request("https://maidan.world/x", { method: "POST", body, headers });
  assert.equal(await replayableBody(new Request("https://maidan.world/x")), null);
  assert.equal(await replayableBody(new Request("https://maidan.world/x", { method: "DELETE" })), null);
  const copied = await replayableBody(post("hello", { "content-length": "5" }));
  assert.equal(new TextDecoder().decode(copied), "hello");
  assert.equal(await replayableBody(post("x".repeat(10), { "content-length": String(REPLAY_BODY_LIMIT + 1) })), undefined);
  // A streamed body has no length until it is read.
  const stream = new ReadableStream({ start: (ctl) => { ctl.enqueue(new TextEncoder().encode("hi")); ctl.close(); } });
  assert.equal(await replayableBody(new Request("https://maidan.world/x", { method: "POST", body: stream, duplex: "half" })), undefined);
});

test("the Worker holds every request it hands the server, not the status page", () => {
  const worker = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.match(worker, /sendHeld\(\{/);
  assert.match(worker, /probe: \(\) => getContainer\(env\.MAIDAN\)\.fetch\("http:\/\/maidan\/health"\)/);
  assert.doesNotMatch(worker, /statusResponse\(request, \(\) => sendHeld/);
});
