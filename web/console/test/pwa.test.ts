import assert from "node:assert/strict";
import { test } from "node:test";
import { SHELL_SCOPE, cacheable, pushPayload, shellKey } from "../src/pwa/policy.ts";
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
