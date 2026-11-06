import assert from "node:assert/strict";
import { test } from "node:test";
import { ApiError, failure, humanError, problemDetail, wsUrl } from "../src/core/api.ts";
import { clientSetup } from "../src/core/addAgent.ts";
import { TOKEN_PLACEHOLDER, snippets } from "../src/core/connect.ts";
import { writeQuery } from "../src/core/boardQuery.ts";
import { applicationServerKey, disablePush, enablePush, type PushDeps } from "../src/pwa/push.ts";

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

test("a nested error still names the capability, and Tokens lives under Tools", async () => {
  await assert.rejects(
    () => failure(json(403, { error: { message: "needs capability workspace:write" } }), "bearer"),
    (e: unknown) => e instanceof ApiError && /workspace:write/.test(e.message) && /Tools, then Tokens/.test(e.message) && !/object Object/.test(e.message) && !/\bin Tokens\b/.test(e.message.replace("Tools, then Tokens", "")),
  );
  assert.match(humanError(403, "needs capability thread:transition", "bearer"), /Tools, then Tokens/);
  assert.equal(problemDetail({ error: { detail: "  needs capability artifact:upload  " } }), "  needs capability artifact:upload  ");
  assert.equal(problemDetail({ message: "plain" }), "plain");
  assert.equal(problemDetail({ error: { message: "" } }), null);
  assert.equal(problemDetail(null), null);
});

test("an empty base still makes a websocket url in the browser", () => {
  assert.equal(wsUrl("https://maidan.example/"), "wss://maidan.example/ws/subscribe");
  assert.equal(wsUrl("http://127.0.0.1:8080"), "ws://127.0.0.1:8080/ws/subscribe");
  const previous = globalThis.window;
  globalThis.window = { location: { origin: "https://maidan.example" } } as unknown as Window & typeof globalThis.window;
  try {
    assert.equal(wsUrl(""), "wss://maidan.example/ws/subscribe");
  } finally {
    globalThis.window = previous;
  }
});

test("a filtered board link keeps its channel, and a bare channel does not", () => {
  assert.equal(writeQuery({ task: null, channel: "c1", agent: null, state: null, q: "  " }), "#/board");
  const filtered = writeQuery({ task: null, channel: "c2", agent: null, state: "open", q: "" });
  assert.match(filtered, /channel=c2/);
  assert.match(filtered, /state=open/);
  const searched = writeQuery({ task: null, channel: "c2", agent: "m1", state: null, q: "login" });
  assert.match(searched, /channel=c2/);
  assert.match(searched, /agent=m1/);
  assert.match(searched, /q=login/);
  assert.match(writeQuery({ task: "t1", channel: "c1", agent: null, state: null, q: "" }), /channel=c1&task=t1/);
});

test("an existing token can set up Codex, and a blank channel is not quoted", () => {
  const rows = snippets("https://m.example");
  const token = rows.filter((s) => s.token).map((s) => s.id);
  assert.deepEqual(token, ["json", "claude", "codex"]);
  const codex = rows.find((s) => s.id === "codex");
  assert.equal(codex?.title, "Codex (~/.codex/config.toml, with a token)");
  assert.match(codex?.text ?? "", new RegExp(`export MAIDAN_TOKEN=${TOKEN_PLACEHOLDER}`));
  assert.match(codex?.text ?? "", /tool_timeout_sec = 330/);
  assert.match(codex?.text ?? "", /bearer_token_env_var = "MAIDAN_TOKEN"/);
  const loop = clientSetup("loop", "https://m.example", "secret", "").map((s) => s.text).join("\n");
  assert.match(loop, /claim_next_thread\("CHANNEL_ID"\)/);
  assert.equal(loop.includes('claim_next_thread("")'), false);
});

test("push survives a trimmed key, a rejected permission, and a revoked subscription", async () => {
  assert.deepEqual([...applicationServerKey("  AQID_-8  ")], [...applicationServerKey("AQID_-8")]);
  const deps = {
    permission: () => "default",
    requestPermission: async () => {
      throw new Error("webview");
    },
    pushManager: async () => ({ getSubscription: async () => null, subscribe: async () => { throw new Error("no"); } }),
    vapidKey: async () => ({ public_key: "AQID" }),
    save: async () => {},
  } satisfies PushDeps;
  assert.deepEqual(await enablePush("mem", deps), { registered: false, reason: "permission_denied" });
  await assert.doesNotReject(() => disablePush({
    pushManager: async () => ({
      getSubscription: async () => { throw new Error("revoked"); },
      subscribe: async () => { throw new Error("no"); },
    }),
  }, async () => { throw new Error("no"); }));
  await assert.doesNotReject(() => disablePush({
    pushManager: async () => ({
      getSubscription: async () => ({
        endpoint: "https://push.example/1",
        toJSON: () => ({}),
        unsubscribe: async () => { throw new Error("gone"); },
      }),
      subscribe: async () => { throw new Error("no"); },
    }),
  }, async () => {}));
});
