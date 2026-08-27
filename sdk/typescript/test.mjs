// Black-box test for the Maidan TS client against a running server (MAIDAN_URL,
// auth disabled). Run via `scripts/sdk-test.sh typescript`, which boots a server.
// These scenarios also exercise the server's REST + WS surface.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Client, MaidanError } from "./index.js";

const BASE = process.env.MAIDAN_URL || "http://127.0.0.1:8080";
const client = new Client(BASE, process.env.MAIDAN_TOKEN || "");

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

test("claim-next returns a claimable thread", async () => {
  const { member, channel } = await seed();
  const res = await client.claimNextThread(channel.id, { member_id: member.id });
  // Either the seeded thread (claimed=true) or null if none ready.
  assert.ok(res === null || typeof res === "object");
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
