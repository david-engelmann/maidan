// Black-box tests against the authenticated server from scripts/sdk-test.sh.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  BadRequestError,
  Client,
  ConflictError,
  ForbiddenError,
  MaidanError,
  NotFoundError,
  UnauthorizedError,
  eventType,
  parseRoomLsn,
} from "./index.js";

const BASE = process.env.MAIDAN_URL || "http://127.0.0.1:8080";
const TOKEN = process.env.MAIDAN_TOKEN || "";
const WORKSPACE = process.env.MAIDAN_WORKSPACE || "";
const client = new Client(BASE, TOKEN);

// The response models live only in index.d.ts, so that file is what these tests
// hold the live server to: every member a response carries must be declared on
// its interface, and every required member must be present.
const MODELS = (() => {
  const src = readFileSync(new URL("./index.d.ts", import.meta.url), "utf8");
  const out = {};
  for (const m of src.matchAll(/export interface (\w+)(?: extends (\w+))? \{([\s\S]*?)\n\}/g)) {
    const fields = {};
    for (const line of m[3].split("\n")) {
      const f = line.match(/^ {2}([$\w]+)(\?)?:/);
      if (f) fields[f[1]] = !f[2];
    }
    out[m[1]] = { base: m[2], fields };
  }
  return out;
})();

function assertShape(value, model) {
  assert.ok(value && typeof value === "object", `${model}: expected an object, got ${value}`);
  const fields = {};
  for (let name = model; name; name = MODELS[name].base) {
    assert.ok(MODELS[name], `no interface ${name} in index.d.ts`);
    Object.assign(fields, MODELS[name].fields);
  }
  for (const key of Object.keys(value)) {
    assert.ok(key in fields, `the server sent ${model}.${key}, which index.d.ts does not declare`);
  }
  for (const [key, required] of Object.entries(fields)) {
    if (required) assert.ok(key in value, `${model}.${key} is required but the server omitted it`);
  }
  return value;
}

const eachShape = (rows, model) => {
  assert.ok(Array.isArray(rows), `${model}[]: expected an array`);
  rows.forEach((row) => assertShape(row, model));
  return rows;
};

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

test("threads.listAll walks every page of a channel", async () => {
  const { channel, thread } = await seed();
  const made = [thread.id];
  for (let i = 0; i < 4; i++) made.push((await client.threads.create(channel.id, `t${i}`)).id);
  const seen = [];
  for await (const t of client.threads.listAll(channel.id, { pageSize: 2 })) seen.push(t.id);
  assert.deepEqual(seen.sort(), made.sort());
});

test("every documented operation returns its declared model", async () => {
  const { channel, thread } = await seed();
  const me = await (await fetch(`${BASE}/me`, { headers: { authorization: `Bearer ${TOKEN}` } })).json();

  assertShape(await client.workspaces.get(WORKSPACE), "Workspace");
  eachShape(await client.members.list(WORKSPACE), "Member");
  assertShape(channel, "Channel");
  eachShape(await client.channels.list(WORKSPACE), "Channel");
  assertShape(thread, "Thread");
  assertShape(await client.threads.get(thread.id), "Thread");
  eachShape(await client.threads.list(channel.id), "Thread");

  const msg = assertShape(await client.messages.post(thread.id, "typed"), "Message");
  eachShape(await client.messages.list(thread.id), "Message");

  const art = assertShape(await client.artifacts.upload("typed bytes", "attachment"), "Artifact");
  assertShape(await client.artifacts.meta(art.sha256), "Artifact");
  assert.equal(new TextDecoder().decode(await client.artifacts.get(art.sha256)), "typed bytes");

  const claim = assertShape(await client.claimNextThread(channel.id, { lease_secs: 60 }), "ClaimedThread");
  assertShape(claim.pin, "StrongRef");
  assertShape(await client.renewClaim(claim.id, claim.claim_lease_id, 120), "Thread");

  const result = assertShape(await client.threads.setResult(thread.id, { ok: true }), "ThreadResult");
  assert.deepEqual(result.result, { ok: true });
  assert.equal(result.produced_by, me.member_id);
  assertShape(await client.threads.getResult(thread.id), "ThreadResult");
  const reviewed = assertShape(await client.threads.transition(thread.id, { action: "start_review" }), "Thread");
  assert.equal(reviewed.state, "in_review");

  const ctx = assertShape(await client.threads.context(thread.id), "ThreadContext");
  assertShape(ctx.thread, "ThreadBrief");
  eachShape(ctx.messages, "Message");
  eachShape(ctx.message_edits, "MessageEditView");
  eachShape(ctx.references, "Reference");
  eachShape(ctx.artifacts, "Artifact");
  assert.equal(ctx.state, "in_review");
  assert.ok(ctx.transitions.length > 0, "the start_review transition is in the pack");
  eachShape(ctx.transitions, "ThreadTransition");
  assert.equal(ctx.prefix_sha256.length, 64);
  assert.ok(ctx.messages.some((m) => m.id === msg.id));

  const events = eachShape(await client.workspaces.events(WORKSPACE, { limit: 50 }), "StoredEvent");
  assert.ok(events.length > 0);
  for await (const e of client.workspaces.eventsAll(WORKSPACE, { limit: 25 })) assertShape(e, "StoredEvent");

  const member = assertShape(await client.members.create(WORKSPACE, `typed-${crypto.randomUUID()}`), "Member");
  const minted = assertShape(await client.tokens.mint(WORKSPACE, member.id, ["workspace:read"]), "MintedToken");
  minted.quotas.forEach((q) => assertShape(q, "TokenQuota"));
  eachShape(await client.tokens.list(WORKSPACE, member.id), "TokenSummary");

  const exported = await (
    await fetch(`${BASE}/workspaces/${WORKSPACE}/export`, { headers: { authorization: `Bearer ${TOKEN}` } })
  ).json();
  const imported = assertShape(await client.workspaces.import(exported, "new"), "ImportResult");
  assert.equal(imported.mode, "new");
  assert.notEqual(imported.workspace_id, WORKSPACE, "mode=new remaps ids");
});

test("the server's problem types arrive as their error classes", async () => {
  const { thread } = await seed();
  const expectError = async (call, Cls, status, type) => {
    await assert.rejects(call, (err) => {
      assert.ok(err instanceof Cls, `expected ${Cls.name}, got ${err && err.name}`);
      assert.ok(err instanceof MaidanError);
      assert.equal(err.status, status);
      assert.equal(err.type, `https://maidan.dev/problems/${type}`);
      assert.equal(err.problem.type, err.type, "the raw problem is kept");
      assert.equal(typeof err.title, "string");
      assert.equal(typeof err.detail, "string");
      return true;
    });
  };
  await expectError(() => client.threads.get("00000000-0000-0000-0000-000000000000"), NotFoundError, 404, "not-found");
  await expectError(() => new Client(BASE, "maid_not_a_token").workspaces.get(WORKSPACE), UnauthorizedError, 401, "unauthorized");
  await expectError(() => client.threads.transition(thread.id, { action: "fly" }), BadRequestError, 400, "bad-request");
  // Bootstrap creates only the first workspace; `maidan init` already made it.
  await expectError(() => client.workspaces.create("second"), ForbiddenError, 403, "forbidden");
  const exported = await (
    await fetch(`${BASE}/workspaces/${WORKSPACE}/export`, { headers: { authorization: `Bearer ${TOKEN}` } })
  ).json();
  await expectError(() => client.workspaces.import(exported, "restore"), ConflictError, 409, "conflict");
});
