// Black-box tests against the authenticated server from scripts/sdk-test.sh.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Client, MaidanError, eventType, parseRoomLsn } from "./index.js";

const BASE = process.env.MAIDAN_URL || "http://127.0.0.1:8080";
const TOKEN = process.env.MAIDAN_TOKEN || "";
const WORKSPACE = process.env.MAIDAN_WORKSPACE || "";
const client = new Client(BASE, TOKEN);

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

async function seed() {
  const meResp = await fetch(`${BASE}/me`, {
    headers: { authorization: `Bearer ${TOKEN}` },
  });
  assert.equal(meResp.ok, true);
  const me = await meResp.json();
  const ws = { id: WORKSPACE };
  const member = { id: me.member_id };
  const channel = await client.channels.create(WORKSPACE, `ts-sdk-${crypto.randomUUID()}`);
  const thread = await client.threads.create(channel.id, "kickoff");
  return { ws, member, channel, thread };
}

test("hero loop: post, list, context", async () => {
  const { member, thread } = await seed();
  await client.messages.post(thread.id, "hello from the ts sdk");
  const msgs = await client.messages.list(thread.id);
  assert.ok(msgs.some((m) => m.body === "hello from the ts sdk"), "posted message is listed");
  const ctx = await client.threads.context(thread.id);
  assert.equal(typeof ctx, "object");
});

test("getResult on an unset thread is a 404 MaidanError", async () => {
  // Exercise the result route and client error path before a result exists.
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
  const claim = await client.claimNextThread(channel.id);
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
    lease_secs: 60,
  });
  const renewed = await client.renewClaim(claim.id, claim.claim_lease_id, 600);
  assert.ok(renewed.assignment_expires_at > claim.assignment_expires_at);
});

test("claim-next returns null once the queue is drained", async () => {
  const { member, channel } = await seed();
  await client.claimNextThread(channel.id);
  assert.equal(await client.claimNextThread(channel.id), null);
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
        setTimeout(() => client.messages.post(thread.id, "ws ping"), 100);
        // Safety close.
        setTimeout(() => sub.close(), 5000);
      });
  });
  const event = await received;
  assert.equal(event.kind, "message_posted");
});

test("provisioning seeds a member and mints a scoped token", async () => {
  // The first thing an integrator does after `maidan init`. Both calls were
  // reachable only through the private transport before.
  const handle = `provisioned-${crypto.randomUUID()}`;
  const member = await client.members.create(WORKSPACE, handle);
  assert.equal(member.handle, handle);
  assert.equal(member.kind, "agent");
  const members = await client.members.list(WORKSPACE);
  assert.ok(members.some((m) => m.id === member.id));

  const minted = await client.tokens.mint(WORKSPACE, member.id, ["workspace:read"], {
    label: "scoped worker",
  });
  assert.ok(minted.secret, "the secret is returned once, in the mint response");
  assert.deepEqual(minted.capabilities, ["workspace:read"]);

  const listed = await client.tokens.list(WORKSPACE, member.id);
  assert.ok(listed.some((t) => t.id === minted.id));
  assert.ok(listed.every((t) => t.secret === undefined), "listing never returns a secret");
});
