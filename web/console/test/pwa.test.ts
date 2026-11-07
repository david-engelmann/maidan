import assert from "node:assert/strict";
import { test } from "node:test";
import { NOTIFICATION_TARGET, SHELL_SCOPE, cacheable, notificationTarget, pushPayload, shellKey } from "../src/pwa/policy.ts";
import { openNotification, showPush } from "../src/pwa/sw.ts";
import { applicationServerKey, disablePush, enablePush, type PushDeps, type SubscriptionLike } from "../src/pwa/push.ts";

const O = "https://m.example";
const req = (url: string, extra: Partial<{ method: string; hasAuthorization: boolean }> = {}) => ({ method: "GET", url, origin: O, hasAuthorization: false, ...extra });

test("the worker caches only the static console shell", () => {
  assert.equal(shellKey(req(`${O}/ui/`)), SHELL_SCOPE);
  assert.equal(shellKey(req(`${O}/ui`)), SHELL_SCOPE);
  assert.equal(shellKey(req(`${O}/ui/app.js`)), "/ui/app.js");
  assert.equal(shellKey(req(`${O}/ui/manifest.webmanifest`)), "/ui/manifest.webmanifest");
});

test("the worker never caches API, auth, socket, query or token-bearing requests", () => {
  for (const u of ["/ui/api/me", "/v1/workspaces/w/board", "/auth/session", "/ws", "/ui/unknown.js", "/ui/oauth/consent", "/ui/app.js?v=1"]) {
    assert.equal(shellKey(req(`${O}${u}`)), null, u);
  }
  assert.equal(shellKey(req(`${O}/ui/app.js`, { hasAuthorization: true })), null);
  assert.equal(shellKey(req(`${O}/ui/app.js`, { method: "POST" })), null);
  assert.equal(shellKey(req(`https://evil.example/ui/app.js`)), null);
});

test("only plain same-origin 200s without no-store or private are stored", () => {
  assert.equal(cacheable(200, "basic", "no-cache"), true);
  assert.equal(cacheable(200, "basic", "private, max-age=0"), false);
  assert.equal(cacheable(200, "basic", "no-store"), false);
  assert.equal(cacheable(200, "opaque", null), false);
  assert.equal(cacheable(304, "basic", null), false);
});

test("push payloads fall back like the classic worker", () => {
  assert.deepEqual(pushPayload(null, null), { title: "Maidan", body: "" });
  assert.equal(pushPayload(() => ({ title: "Gate", body: "Needs you", log_id: "l1" }), null).log_id, "l1");
  assert.deepEqual(pushPayload(() => { throw new Error("x"); }, () => "plain"), { title: "Maidan", body: "plain" });
});

test("push payloads keep numeric log ids and humanize only the server's raw body", () => {
  const numeric = pushPayload(() => ({ title: "Maidan", body: "New notification (mention_recorded)", kind: "mention_recorded", log_id: 42, thread_id: "thr_1" }), null);
  assert.equal(numeric.log_id, "42");
  assert.equal(numeric.thread_id, "thr_1");
  assert.equal(numeric.body, "You were mentioned in a thread.");
  assert.equal(pushPayload(() => ({ body: "Needs you", log_id: "l1", kind: "mention_recorded" }), null).body, "Needs you");
  assert.equal(pushPayload(() => ({ body: "", kind: "claim_expired" }), null).body, "A claimed task needs attention.");
  assert.equal(pushPayload(() => ({ body: "New notification (review_submitted)" }), null).body, "Changes were requested on your work.");
  assert.equal(pushPayload(() => ({ body: "New notification (thread_landed)" }), null).body, "A followed task landed.");
  assert.equal(pushPayload(() => ({ body: "New notification (custom_event_kind)" }), null).body, "New notification: custom event kind");
  assert.equal(pushPayload(() => ({ log_id: Number.NaN, body: "Keep" }), null).log_id, undefined);
});

test("notification clicks open a safe console path, the thread, the inbox, or Needs you", () => {
  assert.equal(notificationTarget({ url: "/ui/#/board" }), "/ui/#/board");
  assert.equal(notificationTarget({ url: "https://evil.example", thread_id: "abc" }), "/ui/#/threads/t/abc");
  assert.equal(notificationTarget({ url: "/ui/#/ok\n", thread_id: "../x", kind: "message_posted" }), "/ui/#/tools/inbox");
  assert.equal(notificationTarget({ url: "/ui/#//evil", kind: "approval_requested" }), NOTIFICATION_TARGET);
  assert.equal(notificationTarget({ kind: "claim_expired" }), NOTIFICATION_TARGET);
  assert.equal(notificationTarget(null), NOTIFICATION_TARGET);
});

test("a push is tagged by log id, and a click moves an open console window or opens one", async () => {
  const shown: { title: string; opts: { body?: string; tag?: string; data?: { log_id?: string } } }[] = [];
  const origin = "https://m.example";
  const worker = {
    location: { origin } as Location,
    registration: { showNotification: async (title: string, opts: { body?: string; tag?: string; data?: { log_id?: string } }) => void shown.push({ title, opts }) },
    clients: { claim: async () => undefined, matchAll: async () => [], openWindow: async () => undefined },
    addEventListener: () => undefined,
    skipWaiting: async () => undefined,
  };
  await showPush(worker, { data: { json: () => ({ body: "New notification (message_posted)", kind: "message_posted", log_id: 7 }), text: () => "" }, waitUntil: () => undefined });
  assert.equal(shown[0].opts.tag, "maidan-7");
  assert.equal(shown[0].opts.body, "New message in a followed thread.");
  assert.equal(shown[0].opts.data?.log_id, "7");

  const navigated: string[] = [];
  let focused = 0;
  const client = { url: `${origin}/ui/#/board`, focus: async () => { focused += 1; }, navigate: async (url: string) => { navigated.push(url); client.url = url; } };
  const hosted = { ...worker, clients: { ...worker.clients, matchAll: async () => [client], openWindow: async (url: string) => { navigated.push(`open:${url}`); } } };
  await openNotification(hosted, { notification: { close: () => undefined, data: { kind: "message_posted" } }, waitUntil: () => undefined });
  assert.deepEqual(navigated, [`${origin}/ui/#/tools/inbox`]);
  assert.equal(focused, 1);
  assert.equal(client.url, `${origin}/ui/#/tools/inbox`);

  const opened: string[] = [];
  focused = 0;
  const alone = { ...worker, clients: { ...worker.clients, matchAll: async () => [], openWindow: async (url: string) => { opened.push(url); } } };
  await openNotification(alone, { notification: { close: () => undefined, data: { kind: "approval_requested" } }, waitUntil: () => undefined });
  assert.deepEqual(opened, [`${origin}${NOTIFICATION_TARGET}`]);
  assert.equal(focused, 0);
});

test("applicationServerKey decodes base64url", () => {
  assert.deepEqual([...applicationServerKey("AQID_-8")], [1, 2, 3, 255, 239]);
});

function fakeSub(): SubscriptionLike & { gone: boolean } {
  const s = { gone: false, endpoint: "https://push.example/e1", toJSON: () => ({ endpoint: "https://push.example/e1", keys: { p256dh: "k", auth: "a" } }), unsubscribe: async () => ((s.gone = true), true) };
  return s;
}

function deps(over: Partial<PushDeps> = {}): PushDeps & { saved: unknown[] } {
  const saved: unknown[] = [];
  const sub = fakeSub();
  return {
    saved,
    permission: () => "default",
    requestPermission: async () => "granted",
    pushManager: async () => ({ getSubscription: async () => null, subscribe: async () => sub }),
    vapidKey: async () => ({ public_key: "AQID" }),
    save: async (m, s) => void saved.push([m, s]),
    ...over,
  };
}

test("enablePush subscribes and stores the subscription for the member", async () => {
  const d = deps();
  assert.deepEqual(await enablePush("m1", d), { registered: true, reason: "registered" });
  assert.deepEqual(d.saved, [["m1", { endpoint: "https://push.example/e1", keys: { p256dh: "k", auth: "a" } }]]);
});

test("enablePush names each reason it stops", async () => {
  assert.equal((await enablePush(null, deps())).reason, "no_member");
  assert.equal((await enablePush("m1", deps({ pushManager: async () => null }))).reason, "unsupported");
  assert.equal((await enablePush("m1", deps({ vapidKey: async () => null }))).reason, "vapid_unavailable");
  assert.equal((await enablePush("m1", deps({ vapidKey: async () => ({ public_key: null }) }))).reason, "vapid_unset");
  assert.equal((await enablePush("m1", deps({ requestPermission: async () => "denied" }))).reason, "permission_denied");
  assert.equal((await enablePush("m1", deps({ save: async () => { throw new Error("500"); } }))).reason, "register_failed");
});

test("disablePush forgets the server copy and unsubscribes", async () => {
  const sub = fakeSub();
  const forgot: string[] = [];
  const out = await disablePush({ pushManager: async () => ({ getSubscription: async () => sub, subscribe: async () => sub }) }, async (e) => void forgot.push(e));
  assert.equal(out.reason, "off");
  assert.deepEqual(forgot, ["https://push.example/e1"]);
  assert.equal(sub.gone, true);
});
