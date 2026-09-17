// Black-box test for the Maidan TS client against a running server (MAIDAN_URL,
// auth disabled). Run via `scripts/sdk-test.sh typescript`, which boots a server.
// These scenarios also exercise the server's REST + WS surface.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Client, MaidanError, eventType, parseRoomLsn } from "./index.js";

const BASE = process.env.MAIDAN_URL || "http://127.0.0.1:8080";
const client = new Client(BASE, process.env.MAIDAN_TOKEN || "");

test("parseRoomLsn accepts decimal and rejects WAL", () => {
  assert.equal(parseRoomLsn("42"), 42);
  assert.equal(parseRoomLsn(" 0 "), 0);
  assert.equal(parseRoomLsn("0/3000128"), undefined);
  assert.equal(parseRoomLsn("-1"), undefined);
  assert.equal(eventType("message_posted"), "maidan.event.message_posted/1");
});

test("isCursorTooOld is 409 must_refetch, not a plain conflict", () => {
  const tooOld = new MaidanError(409, {
    type: "https://maidan.dev/problems/cursor-too-old",
    must_refetch: true,
  });
  assert.equal(tooOld.isConflict, true);
  assert.equal(tooOld.isCursorTooOld, true);
  const plain = new MaidanError(409, { type: "https://maidan.dev/problems/conflict" });
  assert.equal(plain.isConflict, true);
  assert.equal(plain.isCursorTooOld, false);
});

// Member creation isn't in the SDK surface (seeded via bootstrap/CLI); the test
// seeds one over the raw bootstrap route.
async function seed() {
  const ws = await client.workspaces.create("ts-sdk");
  const memberResp = await fetch(`${BASE}/workspaces/${ws.id}/members`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ handle: "sdk-agent", kind: "agent" }),
  });
  const member = await memberResp.json();
  const channel = await client.channels.create(ws.id, "general");
  const thread = await client.threads.create(channel.id, "kickoff");
  return { ws, member, channel, thread };
}

test("hero loop: post, list, context", async () => {
  const { member, thread } = await seed();
  await client.messages.post(thread.id, member.id, "hello from the ts sdk");
  const msgs = await client.messages.list(thread.id);
  assert.ok(msgs.some((m) => m.body === "hello from the ts sdk"), "posted message is listed");
  const ctx = await client.threads.context(thread.id);
  assert.equal(typeof ctx, "object");
});

test("getResult on an unset thread is a 404 MaidanError", async () => {
  // A full setResult round-trip needs a real `produced_by` member (auth-enabled;
  // the server's thread_result_e2e proves it). Under the auth-disabled harness the
  // acting member is nil, so here we exercise the result route + client error path.
  const { thread } = await seed();
  await assert.rejects(
    () => client.threads.getResult(thread.id),
    (err) => {
      assert.ok(err instanceof MaidanError);
      assert.equal(err.status, 404);
      return true;
    },
  );
});

test("claim returns the thread flattened, not nested", async () => {
  // The seeded thread is ready, so this claims it. The shape assertions are the
  // point: a nested `thread` key would make every README snippet a silent no-op.
  const { member, channel, thread } = await seed();
  const claim = await client.claimNextThread(channel.id, { member_id: member.id });
  assert.ok(claim, "a freshly seeded ready thread should be claimable");
  assert.ok(!("thread" in claim), "thread fields are flattened, not nested");
  assert.equal(claim.id, thread.id);
  assert.equal(claim.assignee_id, member.id);
  assert.ok(claim.claim_lease_id, "the fencing token renewClaim needs");
  assert.ok(claim.pin.uri && claim.pin.content_hash);
});

test("renewClaim extends the lease with the fencing token", async () => {
  const { member, channel } = await seed();
  const claim = await client.claimNextThread(channel.id, {
    member_id: member.id,
    lease_secs: 60,
  });
  const renewed = await client.renewClaim(claim.id, member.id, claim.claim_lease_id, 600);
  assert.ok(renewed.assignment_expires_at > claim.assignment_expires_at);
});

test("claim-next returns null once the queue is drained", async () => {
  const { member, channel } = await seed();
  await client.claimNextThread(channel.id, { member_id: member.id });
  assert.equal(await client.claimNextThread(channel.id, { member_id: member.id }), null);
});

test("errors surface status + body", async () => {
  await assert.rejects(
    () => client.threads.get("00000000-0000-0000-0000-000000000000"),
    (err) => {
      assert.ok(err instanceof MaidanError);
      assert.ok(err.status >= 400);
      return true;
    },
  );
});

test("subscribe delivers a posted message (WS)", { skip: typeof WebSocket === "undefined" ? "no global WebSocket (Node <22)" : false }, async () => {
  const { ws, member, thread } = await seed();
  const received = new Promise((resolve) => {
    client
      .subscribe({ workspace_id: ws.id, kinds: ["message_posted"] }, (e) => {
        if (e.thread_id === thread.id) resolve(e);
      })
      .then((sub) => {
        // Post after the subscription is attached.
        setTimeout(() => client.messages.post(thread.id, member.id, "ws ping"), 100);
        // Safety close.
        setTimeout(() => sub.close(), 5000);
      });
  });
  const event = await received;
  assert.equal(event.kind, "message_posted");
});
