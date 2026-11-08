import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { KEEP, Notices, redact } from "../src/core/notices.ts";
import { CAPS_REQUIRED, PeopleApi, WORKER_PRESET } from "../src/core/people.ts";
import {
  NAME_REQUIRED,
  PEERS_NEED,
  TARGET_REQUIRED,
  ToolsApi,
  capsExceeding,
  deliveryQuery,
  deliveryState,
  emailRefusal,
  formatBytes,
  holds,
  isSha,
  mintAdminRefusal,
  parseCaps,
  previewable,
  scheduleCadence,
  channelFollowLabel,
  channelsToFollow,
  memberChoices,
  memberLabel,
  showsOperator,
  slashRefusal,
} from "../src/core/tools.ts";
import { ROUTES, parseRoute } from "../src/shell/router.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };
type R = { status?: number; body?: unknown };

function fetcher(responses: R[]) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let i = 0;
  const fn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const r = responses[Math.min(i++, responses.length - 1)] ?? {};
    return new Response(r.body === undefined ? null : JSON.stringify(r.body), { status: r.status ?? 200 });
  };
  return { fn, calls };
}
const tools = (responses: R[], creds = session) => {
  const f = fetcher(responses);
  return { api: new ToolsApi(f.fn, () => creds, () => "session"), calls: f.calls };
};
const people = (responses: R[], creds = bearer) => {
  const f = fetcher(responses);
  return { api: new PeopleApi(f.fn, () => creds, () => "token"), calls: f.calls };
};
const SHA = "a".repeat(64);

test("tools and files are routes", () => {
  assert.ok(ROUTES.some((r) => r.id === "tools"));
  assert.ok(ROUTES.some((r) => r.id === "artifacts"));
  assert.equal(parseRoute("#/tools/commands"), "tools");
  assert.equal(parseRoute("#/artifacts"), "artifacts");
});

test("pure helpers", () => {
  assert.equal(deliveryQuery("all", "quarantined"), "kind=all&limit=50&quarantined=true");
  assert.equal(deliveryQuery("webhook", "delivered", 10), "kind=webhook&limit=10&delivered=true");
  assert.equal(deliveryState({ quarantined_at: "x", delivered_at: "y" } as never), "dlq");
  assert.equal(deliveryState({ delivered_at: "y" } as never), "delivered");
  assert.equal(deliveryState({} as never), "pending");
  assert.equal(scheduleCadence({ interval_secs: 86400 } as never), "Every day");
  assert.equal(scheduleCadence({ interval_secs: 7200 } as never), "Every 2 hours");
  assert.equal(scheduleCadence({ interval_secs: 0 } as never), "Runs once");
  assert.equal(slashRefusal("/Deploy", "x"), NAME_REQUIRED);
  assert.equal(slashRefusal("/deploy", " "), TARGET_REQUIRED);
  assert.equal(slashRefusal("/deploy", "https://x"), null);
  assert.equal(emailRefusal("a@b.co"), null);
  assert.ok(emailRefusal("nope"));
  assert.deepEqual(parseCaps(" a, b  a,,c "), ["a", "b", "c"]);
  assert.deepEqual(capsExceeding(["a", "z"], ["a"]), ["z"]);
  assert.deepEqual(capsExceeding(["a"], null), []);
  assert.equal(formatBytes(2048), "2.0 KB");
  assert.equal(formatBytes(null), "");
  assert.equal(formatBytes(-1), "");
  assert.equal(formatBytes(3 * 1024 * 1024), "3.0 MB");
  assert.equal(previewable("image/PNG; x=1"), true);
  assert.equal(previewable("image/svg+xml"), false);
  assert.equal(isSha(SHA), true);
  assert.equal(isSha("abc"), false);
});

test("reads go to the session tree, or the bearer tree with a token", async () => {
  const a = tools([{ body: { count: 3 } }]);
  assert.equal(await a.api.unread("me"), 3);
  assert.equal(a.calls[0].url, "http://h/ui/api/members/me/notifications/unread-count");
  const b = tools([{ body: [] }], bearer);
  await b.api.commands("w");
  assert.equal(b.calls[0].url, "http://h/workspaces/w/slash-commands");
});

test("a 404 on an optional read is nothing, not an error", async () => {
  const { api } = tools([{ status: 404, body: { error: "not_found" } }]);
  assert.equal(await api.email("me"), null);
  const m = tools([{ status: 404 }]);
  assert.equal(await m.api.artifact(SHA), null);
});

test("no delivery email set is a 200 with null, read as no address", async () => {
  const { api } = tools([{ status: 200, body: null }]);
  assert.equal(await api.email("me"), null);
  const set = tools([{ body: { member_id: "me", email: "a@b.co", updated_at: "2026-11-02T08:00:00Z" } }]);
  assert.equal(await set.api.email("me"), "a@b.co");
});

test("writes carry an idempotency key and a 204 is fine", async () => {
  const { api, calls } = tools([{ status: 204 }]);
  await api.link("t1", SHA);
  assert.equal(calls[0].init?.method, "PUT");
  assert.ok(calls[0].url.endsWith(`/threads/t1/artifacts/${SHA}`));
  assert.ok(new Headers(calls[0].init?.headers).get("Idempotency-Key"));
});

test("register refuses a bad name before sending, and returns the secret", async () => {
  const bad = tools([{}]);
  await assert.rejects(bad.api.register("w", { name: "", description: "", handlerKind: "http", handlerTarget: "x" }), { message: NAME_REQUIRED });
  assert.equal(bad.calls.length, 0);
  const ok = tools([{ status: 201, body: { id: "c", secret: "s3cr3t" } }]);
  assert.deepEqual(await ok.api.register("w", { name: "/ship", description: "", handlerKind: "http", handlerTarget: "https://x" }), { secret: "s3cr3t" });
  assert.deepEqual(JSON.parse(String(ok.calls[0].init?.body)), { name: "ship", description: null, handler_kind: "http", handler_target: "https://x" });
});

test("server-wide reindex and audit need a bearer token", async () => {
  const s = tools([{}]);
  await assert.rejects(s.api.reindex(null));
  await assert.rejects(s.api.audit(10));
  assert.equal(s.calls.length, 0);
  const w = tools([{ body: { id: "j", status: "queued" } }]);
  assert.equal((await w.api.reindex("w")).id, "j");
  assert.ok(w.calls[0].url.endsWith("/ui/api/operator/reindex-embeddings"));
});

test("events filter by kind and show the newest first", async () => {
  const { api, calls } = tools([{ body: [{ kind: "a", id: 3 }, { kind: "b", id: 2 }, { kind: "a", id: 1 }] }]);
  assert.deepEqual((await api.events("w", "a")).map((e) => (e as { id: number }).id), [3, 1]);
  assert.match(calls[0].url, /[?&]order=desc(&|$)/);
});

test("recent events read one newest-first page and hand it back oldest first", async () => {
  const { api, calls } = tools([{ body: [{ id: 9 }, { id: 8 }, { id: 7 }] }]);
  assert.deepEqual((await api.recentEvents("w")).map((e) => e.id), [7, 8, 9]);
  assert.equal(calls.length, 1);
  assert.match(calls[0].url, /\/workspaces\/w\/events\?order=desc&limit=200$/);
});

test("a 422 shows the server's own sentence", async () => {
  const { api } = tools([{ status: 422, body: { message: "Name taken." } }]);
  await assert.rejects(api.register("w", { name: "x", description: "", handlerKind: "http", handlerTarget: "y" }), { message: "Name taken." });
});

test("notices keep the newest, and hide anything token-shaped", () => {
  const n = new Notices();
  let heard = 0;
  n.subscribe(() => heard++);
  for (let i = 0; i < KEEP + 5; i++) n.push(`n${i}`);
  assert.equal(n.snapshot().length, KEEP);
  assert.equal(n.snapshot()[0].text, `n${KEEP + 4}`);
  assert.equal(heard, KEEP + 5);
  const tok = "mdn_" + "Zx9".repeat(12);
  assert.equal(redact(`made ${tok}`), "made [hidden]");
  assert.equal(redact(SHA), SHA);
  assert.equal(redact("123e4567-e89b-12d3-a456-426614174000"), "123e4567-e89b-12d3-a456-426614174000");
});

test("mint sends the preset or the list, and hands the secret back without logging it", async () => {
  const logged: unknown[] = [];
  const orig = [console.log, console.info, console.debug, console.warn, console.error];
  console.log = console.info = console.debug = console.warn = console.error = (...a: unknown[]) => void logged.push(a);
  try {
    const p = people([{ status: 201, body: { id: "t9", secret: "sekret-value", capabilities: [] } }]);
    assert.deepEqual(await p.api.mint("w", "m", { label: "ci", capabilities: [], preset: WORKER_PRESET }), { id: "t9", secret: "sekret-value" });
    assert.equal(p.calls[0].url, "http://h/workspaces/w/members/m/tokens");
    assert.equal(p.calls[0].init?.method, "POST");
    assert.deepEqual(JSON.parse(String(p.calls[0].init?.body)), { label: "ci", capability_set: WORKER_PRESET, capabilities: [] });
    const q = people([{ status: 201, body: { id: "t", secret: "s" } }]);
    await q.api.mint("w", "m", { label: " ", capabilities: ["thread:read"], preset: null });
    assert.deepEqual(JSON.parse(String(q.calls[0].init?.body)), { label: null, capabilities: ["thread:read"] });
    const r = people([{}]);
    await assert.rejects(r.api.mint("w", "m", { label: "", capabilities: [], preset: null }), { message: CAPS_REQUIRED });
    assert.equal(r.calls.length, 0);
  } finally {
    [console.log, console.info, console.debug, console.warn, console.error] = orig;
  }
  assert.deepEqual(logged, []);
});

test("rotate and revoke hit the token tree; a session cannot", async () => {
  const p = people([{ body: { id: "t1", secret: "new" } }, { status: 204 }]);
  assert.deepEqual(await p.api.rotate("t1"), { id: "t1", secret: "new" });
  await p.api.revoke("t1");
  assert.equal(p.calls[0].url, "http://h/tokens/t1/rotate");
  assert.equal(p.calls[1].url, "http://h/tokens/t1");
  assert.equal(p.calls[1].init?.method, "DELETE");
  const s = people([{}], session);
  await assert.rejects(s.api.revoke("t1"));
  assert.equal(s.calls.length, 0);
  const bad = people([{ body: { id: "t1" } }]);
  await assert.rejects(bad.api.rotate("t1"));
});

test("a credential without federation:admin reads peers as not allowed, and a real failure still fails", async () => {
  const refused = tools([{ status: 403, body: { error: "needs capability federation:admin" } }]);
  assert.deepEqual(await refused.api.peers("w"), { allowed: false });
  const ok = tools([{ body: [{ id: "p1", name: "east", base_url: "https://e.example" }] }]);
  assert.deepEqual(await ok.api.peers("w"), { allowed: true, value: [{ id: "p1", name: "east", base_url: "https://e.example" }] });
  await assert.rejects(tools([{ status: 500 }]).api.peers("w"));
  assert.match(PEERS_NEED, /federation:admin/);
});

test("member and channel pickers speak in names, and hide channels already followed", () => {
  assert.equal(memberLabel({ id: "m1", name: "Rae Reviewer", handle: "rae" }), "Rae Reviewer (@rae)");
  assert.equal(memberLabel({ id: "m2", name: "rae", handle: "@rae" }), "rae");
  assert.equal(memberLabel({ id: "abcd1234ef", name: "  ", handle: "" }), "abcd1234");
  assert.deepEqual(memberChoices([
    { id: "b", name: "Deployer", handle: "deployer" },
    { id: "a", name: "Ada", handle: "ada" },
  ]), [
    { id: "", label: "Anyone" },
    { id: "a", label: "Ada (@ada)" },
    { id: "b", label: "Deployer (@deployer)" },
  ]);
  const channels = [{ id: "c1", name: "general" }, { id: "c2", name: "ops" }, { id: "c3", name: "  " }];
  assert.equal(channelFollowLabel("c1", channels), "#general");
  assert.equal(channelFollowLabel("missing", channels), "missing");
  assert.equal(channelFollowLabel("c3", channels), "c3");
  assert.deepEqual(channelsToFollow(channels, ["c1", ""]).map((c) => c.id), ["c2"]);
});

test("operator and federation panels are offered only to a credential that holds them", () => {
  assert.equal(showsOperator(null), false);
  assert.equal(showsOperator({ capabilities: ["workspace:read", "workspace:write", "token:admin"] }), false);
  assert.equal(showsOperator({ capabilities: ["operator:global"] }), true);
  assert.equal(showsOperator({ capabilities: ["audit:read-global"] }), true);
  assert.equal(holds({ capabilities: ["federation:admin"] }, "federation:admin"), true);
  assert.equal(holds(null, "federation:admin"), false);
});

test("a refused first-admin mint says what to do instead, never the server's setting", () => {
  assert.match(mintAdminRefusal("workspace already has a token:admin holder"), /already has an admin/);
  assert.match(mintAdminRefusal("a member who joined by invite cannot mint the first admin token"), /joined by invite/);
  assert.match(mintAdminRefusal("a session made from a token cannot mint the first admin token"), /signed in with a token/);
  for (const d of ["first-admin session mint is disabled (MAIDAN_OIDC_FIRST_ADMIN)", "OIDC is not enabled", null]) {
    assert.doesNotMatch(mintAdminRefusal(d), /MAIDAN_|OIDC/);
  }
});
