// Unit tests for the problem-type error classes and forward-compatible
// decoding, over a fake fetch (no server needed).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  Client,
  CursorTooOldError,
  MaidanError,
  NotFoundError,
  OverloadedError,
  PROBLEM_BASE,
  PROBLEM_TYPES,
  UnknownProblemError,
  problemError,
} from "./index.js";

function answering(status, body, headers = {}) {
  const fetch = async () =>
    new Response(typeof body === "string" ? body : JSON.stringify(body), {
      status,
      headers: { "content-type": "application/problem+json", ...headers },
    });
  return new Client("http://x", "t", { fetch, maxRetries: 0 });
}

const SERVER_TYPES = [
  "not-found",
  "method-not-allowed",
  "conflict",
  "bad-request",
  "unauthorized",
  "invalid-signature",
  "forbidden",
  "payload-too-large",
  "unsupported-media-type",
  "rate-limited",
  "bad-gateway",
  "internal",
  "overloaded",
  "idempotency-key-reused",
  "idempotency-key-in-flight",
  "cursor-too-old",
  "event-log-broken",
];

test("every problem type the server documents has its own error class", () => {
  assert.deepEqual(
    Object.keys(PROBLEM_TYPES).sort(),
    SERVER_TYPES.map((t) => PROBLEM_BASE + t).sort(),
  );
  const classes = new Set(Object.values(PROBLEM_TYPES));
  assert.equal(classes.size, SERVER_TYPES.length, "no two types share a class");
  for (const [type, Cls] of Object.entries(PROBLEM_TYPES)) {
    const err = problemError(418, { type, title: "T", status: 418, detail: "d" });
    assert.ok(err instanceof Cls && err instanceof MaidanError);
    assert.notEqual(Cls, UnknownProblemError);
    assert.equal(err.name, Cls.name);
  }
});

test("a response carries status, type, title, detail and the raw problem", async () => {
  const body = {
    type: `${PROBLEM_BASE}not-found`,
    title: "Not Found",
    status: 404,
    detail: "the requested resource does not exist",
    trace: "a member added later",
  };
  await assert.rejects(answering(404, body).threads.get("t1"), (err) => {
    assert.ok(err instanceof NotFoundError);
    assert.equal(err.status, 404);
    assert.equal(err.type, body.type);
    assert.equal(err.title, "Not Found");
    assert.equal(err.detail, body.detail);
    assert.deepEqual(err.problem, body);
    assert.match(err.message, /HTTP 404: the requested resource does not exist/);
    return true;
  });
});

test("an unknown problem type falls back to UnknownProblemError", async () => {
  const type = `${PROBLEM_BASE}added-next-year`;
  await assert.rejects(answering(409, { type, title: "New", status: 409, detail: "d" }).threads.get("t"), (err) => {
    assert.ok(err instanceof UnknownProblemError && err instanceof MaidanError);
    assert.equal(err.type, type);
    assert.equal(err.isConflict, true);
    return true;
  });
});

test("a body that is not a problem is UnknownProblemError with its text as detail", async () => {
  await assert.rejects(answering(502, "<html>bad gateway</html>").threads.get("t"), (err) => {
    assert.ok(err instanceof UnknownProblemError);
    assert.equal(err.type, undefined);
    assert.equal(err.problem, undefined);
    assert.equal(err.detail, "<html>bad gateway</html>");
    return true;
  });
});

test("cursor-too-old carries the snapshot to refetch", async () => {
  const body = {
    type: `${PROBLEM_BASE}cursor-too-old`,
    title: "Cursor Too Old",
    status: 409,
    detail: "must refetch",
    must_refetch: true,
    snapshot: "/workspaces/w/snapshot",
  };
  await assert.rejects(answering(409, body).workspaces.events("w"), (err) => {
    assert.ok(err instanceof CursorTooOldError);
    assert.equal(err.isCursorTooOld, true);
    assert.equal(err.snapshot, "/workspaces/w/snapshot");
    return true;
  });
});

test("Retry-After is kept on an overloaded 503", async () => {
  const body = { type: `${PROBLEM_BASE}overloaded`, title: "Service Unavailable", status: 503, detail: "busy" };
  await assert.rejects(answering(503, body, { "retry-after": "7" }).threads.get("t"), (err) => {
    assert.ok(err instanceof OverloadedError);
    assert.equal(err.retryAfter, 7);
    return true;
  });
});

test("members a model does not declare do not break a response", async () => {
  const fetch = async () =>
    new Response(JSON.stringify({ id: "t1", channel_id: "c1", state: "blocked_on_mars", created_at: "x", updated_at: "x", novel: { deep: 1 } }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  const thread = await new Client("http://x", "t", { fetch }).threads.get("t1");
  assert.equal(thread.id, "t1");
  assert.equal(thread.state, "blocked_on_mars");
  assert.deepEqual(thread.novel, { deep: 1 });
});
