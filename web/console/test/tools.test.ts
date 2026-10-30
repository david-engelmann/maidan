import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { KEEP, Notices, redact } from "../src/core/notices.ts";
import { CAPS_REQUIRED, PeopleApi, WORKER_PRESET } from "../src/core/people.ts";
import {
  NAME_REQUIRED,
  TARGET_REQUIRED,
  ToolsApi,
  capsExceeding,
  deliveryQuery,
  deliveryState,
  emailRefusal,
  formatBytes,
  isSha,
  parseCaps,
  previewable,
  scheduleCadence,
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
  const { api } = tools([{ body: [{ kind: "a", id: 1 }, { kind: "b", id: 2 }, { kind: "a", id: 3 }] }]);
  assert.deepEqual((await api.events("w", "a")).map((e) => (e as { id: number }).id), [3, 1]);
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
