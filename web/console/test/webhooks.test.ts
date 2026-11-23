import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { EVENT_KINDS } from "../src/core/tools.ts";
import {
  WEBHOOKS_NEED_TOKEN,
  WEBHOOKS_NO_READ,
  WEBHOOK_CREATE_UNAVAILABLE,
  WEBHOOK_SECRET_NOTE,
  WEBHOOK_URL_RULE,
  WebhooksApi,
  eventWord,
  webhookBody,
  webhookName,
  type Webhook,
} from "../src/core/webhooks.ts";

const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };
const cookieToken: Credentials = { base: "http://h", pastedToken: "", tokenSession: true, sessionMemberId: "me" };
const person: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };

function api(responses: Array<{ status?: number; body?: unknown }>, creds = bearer) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let i = 0;
  const fn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const r = responses[Math.min(i++, responses.length - 1)];
    return new Response(r.status === 204 ? null : JSON.stringify(r.body ?? {}), { status: r.status ?? 200, headers: { "content-type": "application/json" } });
  };
  return { api: new WebhooksApi(fn, () => creds, () => "bearer"), calls };
}

const hook = (over: Partial<Webhook> = {}): Webhook => ({
  id: "w1",
  url: "https://example.com/maidan",
  label: "primary",
  event_kinds: ["message_posted", "thread_result_set"],
  enabled: true,
  created_at: "2026-11-23T16:00:00Z",
  revoked_at: null,
  ...over,
});

test("event kinds read as words, and a webhook is named by its label, else its URL", () => {
  assert.equal(eventWord("thread_result_set"), "Thread result set");
  assert.equal(eventWord("claim_expired"), "Claim expired");
  assert.equal(eventWord(""), "");
  assert.equal(webhookName(hook()), "primary");
  assert.equal(webhookName(hook({ label: "  " })), "https://example.com/maidan");
  assert.equal(webhookName(hook({ label: null })), "https://example.com/maidan");
});

test("the body trims the URL and leaves out an empty label", () => {
  assert.deepEqual(webhookBody({ url: " https://example.com/x ", label: " ", event_kinds: ["message_posted"] }), { url: "https://example.com/x", event_kinds: ["message_posted"] });
  assert.deepEqual(webhookBody({ url: "https://example.com/x", label: " ops ", event_kinds: ["thread_ready"] }), { url: "https://example.com/x", label: "ops", event_kinds: ["thread_ready"] });
});

test("nothing a person reads has an em dash, a semicolon or a colon in running prose", () => {
  for (const w of [WEBHOOKS_NEED_TOKEN, WEBHOOKS_NO_READ, WEBHOOK_URL_RULE, WEBHOOK_SECRET_NOTE, WEBHOOK_CREATE_UNAVAILABLE]) assert.doesNotMatch(w.replace(/\b[a-z]+:[a-z]+\b/g, "capability"), /[—;:]/, w);
  // The signature words are the ones the server sends.
  assert.match(WEBHOOK_SECRET_NOTE, /X-Maidan-Signature/);
  assert.match(WEBHOOK_SECRET_NOTE, /HMAC-SHA256/);
  assert.match(WEBHOOK_SECRET_NOTE, /sha256=/);
});

test("the event kinds on offer are the console's own list", () => {
  assert.ok(EVENT_KINDS.length > 0);
  for (const k of EVENT_KINDS) assert.match(k, /^[a-z]+(_[a-z]+)*$/);
});

test("the list is a quiet refusal on 403, and the webhooks otherwise, on the token's route", async () => {
  const no = api([{ status: 403, body: { detail: "needs workspace:read" } }]);
  assert.deepEqual(await no.api.list("w"), { allowed: false });
  assert.equal(no.calls[0].url, "http://h/workspaces/w/webhooks");
  assert.equal(new Headers(no.calls[0].init?.headers).get("Authorization"), "Bearer tok");
  const yes = api([{ body: [hook()] }], cookieToken);
  const read = await yes.api.list("w");
  assert.equal(read.allowed && read.value[0].id, "w1");
  assert.equal(yes.calls[0].url, "http://h/workspaces/w/webhooks", "a session made from a token uses the same route");
  await assert.rejects(api([{ status: 500 }]).api.list("w"), /The server hit an error/);
});

test("a browser sign-in is refused before anything is sent, because the routes take a token", async () => {
  const p = api([{ body: [] }], person);
  await assert.rejects(p.api.list("w"), /needs a bearer token/);
  await assert.rejects(p.api.create("w", { url: "https://example.com", event_kinds: ["message_posted"] }), /needs a bearer token/);
  await assert.rejects(p.api.revoke("w", "w1"), /needs a bearer token/);
  assert.equal(p.calls.length, 0);
});

test("creating is a POST of the trimmed body that answers with the webhook and its secret", async () => {
  const s = api([{ status: 201, body: { webhook: hook(), secret: "mdn_signing_fixture" } }]);
  const made = await s.api.create("w", { url: " https://example.com/maidan ", label: "primary", event_kinds: ["message_posted"] });
  assert.equal(made.secret, "mdn_signing_fixture");
  assert.equal(made.webhook.id, "w1");
  assert.equal(s.calls[0].url, "http://h/workspaces/w/webhooks");
  assert.equal(s.calls[0].init?.method, "POST");
  assert.deepEqual(JSON.parse(String(s.calls[0].init?.body)), { url: "https://example.com/maidan", label: "primary", event_kinds: ["message_posted"] });
  assert.ok(new Headers(s.calls[0].init?.headers).get("Idempotency-Key"));
});

test("a refusal comes back in the server's own sentence", async () => {
  const url = api([{ status: 400, body: { detail: "egress target is not a public network address" } }]);
  await assert.rejects(url.api.create("w", { url: "http://10.0.0.1", event_kinds: ["message_posted"] }), /egress target is not a public network address/);
  const kind = api([{ status: 400, body: { detail: "unknown event kind: nope" } }]);
  await assert.rejects(kind.api.create("w", { url: "https://example.com", event_kinds: ["nope"] }), /unknown event kind: nope/);
  const admin = api([{ status: 403, body: { detail: "missing capability workspace:admin" } }]);
  await assert.rejects(admin.api.create("w", { url: "https://example.com", event_kinds: ["message_posted"] }), /missing capability workspace:admin/);
  const noDetail = api([{ status: 403, body: {} }]);
  await assert.rejects(noDetail.api.create("w", { url: "https://example.com", event_kinds: ["message_posted"] }), /not allowed to do this/);
});

test("a 500 on create says what usually causes it, since the server's own reason is not sent", async () => {
  const a = api([{ status: 500, body: { detail: "the server failed while handling this request" } }]);
  await assert.rejects(a.api.create("w", { url: "https://example.com", event_kinds: ["message_posted"] }), /Creating one needs FEDERATION_ENCRYPTION_KEY/);
});

test("revoking is a DELETE on the webhook's id that answers 204", async () => {
  const { api: a, calls } = api([{ status: 204 }]);
  await a.revoke("w", "w1");
  assert.equal(calls[0].url, "http://h/workspaces/w/webhooks/w1");
  assert.equal(calls[0].init?.method, "DELETE");
  assert.ok(new Headers(calls[0].init?.headers).get("Idempotency-Key"));
  await assert.rejects(api([{ status: 403, body: { detail: "missing capability workspace:write" } }]).api.revoke("w", "w1"), /workspace:write/);
  await assert.rejects(api([{ status: 404 }]).api.revoke("w", "w1"), /Not found/);
});
