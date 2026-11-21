// The demo's one way out: advice on a hand-off goes from the container to
// this Worker and on to TypeSafe with the Worker's key. The container holds
// no key and reaches nothing else.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { demoEnv } from "../src/demo-env.js";
import {
  JEV_INTERNAL_HOST,
  JEV_KEY_VAR,
  JEV_LIMIT_BINDING,
  JEV_MAX_BODY_BYTES,
  JEV_PLACEHOLDER_KEY,
  JEV_SERVER_ENV,
  RELAYS,
  adviceProviders,
  adviceServerEnv,
  forwardAdvice,
  forwardJev,
  jevEnabled,
  jevOutboundByHost,
} from "../src/demo-jev.js";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const demoWorker = read("../src/demo.js");
const wrangler = JSON.parse(
  read("../wrangler.demo.jsonc")
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n"),
);

const KEY = "ts_live_fixture";
const question = JSON.stringify({ model: "jev-latest", state: { task: {}, result: {} }, questions: {} });

function upstream() {
  const calls = [];
  const fn = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify({ model: "jev-1.14.0", answers: {}, usage: { input_tokens: 1, output_tokens: 1 } }), { status: 200 });
  };
  return { calls, fn };
}

const ask = (init = {}) =>
  new Request(`http://${JEV_INTERNAL_HOST}/v1/systemone`, {
    method: "POST",
    headers: { authorization: `Bearer ${JEV_PLACEHOLDER_KEY}`, "content-type": "application/json", cookie: "x=1" },
    body: question,
    ...init,
  });

test("without a key on the Worker the demo's server is not told about advice at all", () => {
  assert.equal(jevEnabled({}), false);
  assert.equal(jevEnabled({ [JEV_KEY_VAR]: "  " }), false);
  const env = demoEnv(undefined, {});
  for (const name of Object.keys(JEV_SERVER_ENV)) assert.equal(env[name], undefined, name);
});

test("with a key the server is pointed at the Worker and never given the key", () => {
  const env = demoEnv(undefined, { [JEV_KEY_VAR]: KEY });
  assert.equal(env.MAIDAN_JEV_LAND_GATE_ENABLED, "1");
  assert.equal(env.MAIDAN_JEV_BASE_URL, `http://${JEV_INTERNAL_HOST}/`);
  assert.equal(env.MAIDAN_INTERNAL_EGRESS_HOSTS, JEV_INTERNAL_HOST);
  assert.equal(env.TYPESAFE_API_KEY, JEV_PLACEHOLDER_KEY);
  assert.ok(!JSON.stringify(env).includes(KEY), "the real key stays on the Worker");
  // The rest of the demo's settings are untouched.
  assert.equal(env.MAIDAN_DEMO_SANDBOX, "1");
  assert.equal(env.MAIDAN_PUBLIC_ORIGIN, "https://try.maidan.world");
});

test("a question is forwarded to TypeSafe with the Worker's key in place of the container's", async () => {
  const up = upstream();
  const res = await forwardJev(ask(), { [JEV_KEY_VAR]: KEY }, up.fn);
  assert.equal(res.status, 200);
  assert.equal(up.calls.length, 1);
  assert.equal(up.calls[0].url, "https://api.typesafe.ai/v1/systemone");
  assert.equal(up.calls[0].init.method, "POST");
  assert.deepEqual(up.calls[0].init.headers, {
    authorization: `Bearer ${KEY}`,
    accept: "application/json",
    "content-type": "application/json",
  });
  assert.equal(new TextDecoder().decode(up.calls[0].init.body), question);
});

test("a body is read no further than the cap, whatever its Content-Length says", async () => {
  const up = upstream();
  let pulled = 0;
  let stopped = false;
  const chunk = new Uint8Array(4 * 1024);
  // A stream that would never end, and declares no length.
  const endless = new ReadableStream({
    pull(controller) {
      pulled += 1;
      controller.enqueue(chunk);
    },
    cancel() {
      stopped = true;
    },
  });
  const request = new Request(`http://${JEV_INTERNAL_HOST}/v1/systemone`, { method: "POST", body: endless, duplex: "half" });
  assert.equal(request.headers.get("content-length"), null);
  const res = await forwardJev(request, { [JEV_KEY_VAR]: KEY }, up.fn);
  assert.equal(res.status, 413);
  assert.equal(stopped, true, "the rest of the body is dropped, not read");
  assert.ok(pulled * chunk.byteLength <= JEV_MAX_BODY_BYTES + 3 * chunk.byteLength, `${pulled} chunks read`);
  assert.equal(up.calls.length, 0);
  // A body exactly at the cap goes through, byte for byte.
  const atCap = "x".repeat(JEV_MAX_BODY_BYTES);
  assert.equal((await forwardJev(ask({ body: atCap }), { [JEV_KEY_VAR]: KEY }, up.fn)).status, 200);
  assert.equal(up.calls[0].init.body.byteLength, JEV_MAX_BODY_BYTES);
});

test("the key check is forwarded too, and nothing else is", async () => {
  const up = upstream();
  const models = await forwardJev(new Request(`http://${JEV_INTERNAL_HOST}/v1/models`), { [JEV_KEY_VAR]: KEY }, up.fn);
  assert.equal(models.status, 200);
  assert.equal(up.calls[0].url, "https://api.typesafe.ai/v1/models");
  assert.equal(up.calls[0].init.body, undefined);
  const others = [
    ["GET", "/v1/systemone"],
    ["POST", "/v1/models"],
    ["POST", "/v1/keys"],
    ["GET", "/"],
    ["PUT", "/v1/systemone"],
  ];
  for (const [method, path] of others) {
    const init = method === "GET" ? { method } : { method, body: "{}" };
    const res = await forwardJev(new Request(`http://${JEV_INTERNAL_HOST}${path}`, init), { [JEV_KEY_VAR]: KEY }, up.fn);
    assert.equal(res.status, 404, `${method} ${path}`);
  }
  assert.equal(up.calls.length, 1, "a refused call never reaches TypeSafe");
});

test("no key, too large, or too fast: refused here, and TypeSafe is not called", async () => {
  const up = upstream();
  assert.equal((await forwardJev(ask(), {}, up.fn)).status, 503);
  const big = ask({ body: "x".repeat(JEV_MAX_BODY_BYTES + 1) });
  assert.equal((await forwardJev(big, { [JEV_KEY_VAR]: KEY }, up.fn)).status, 413);
  const limited = { [JEV_KEY_VAR]: KEY, [JEV_LIMIT_BINDING]: { limit: async () => ({ success: false }) } };
  const fast = await forwardJev(ask(), limited, up.fn);
  assert.equal(fast.status, 429);
  assert.equal(up.calls.length, 0);
  // Within the limit it goes through, and the limiter is asked once, on one key.
  const asked = [];
  const within = { [JEV_KEY_VAR]: KEY, [JEV_LIMIT_BINDING]: { limit: async (o) => (asked.push(o), { success: true }) } };
  assert.equal((await forwardJev(ask(), within, up.fn)).status, 200);
  assert.deepEqual(asked, [{ key: "demo" }]);
  // The key check is not a question, so it is not counted.
  await forwardJev(new Request(`http://${JEV_INTERNAL_HOST}/v1/models`), within, up.fn);
  assert.equal(asked.length, 1);
});

test("the demo routes only the internal name, keeps the internet off, and exports the proxy", () => {
  assert.deepEqual(Object.keys(jevOutboundByHost), [JEV_INTERNAL_HOST, "openai-decisions.internal", "gemini-decisions.internal", "perplexity-decisions.internal"]);
  // Cloudflare's Clef is the server's to offer, not the demo's to ask.
  assert.ok(!RELAYS.some((relay) => relay.id === "cloudflare" || /cloudflare/.test(relay.origin)));
  assert.match(demoWorker, /enableInternet = false/);
  assert.doesNotMatch(demoWorker, /static\s+outboundByHost\s*=/);
  assert.match(demoWorker, /MaidanDemo\.outboundByHost = jevOutboundByHost;/);
  assert.match(demoWorker, /export \{ ContainerProxy \} from "@cloudflare\/containers";/);
  assert.doesNotMatch(demoWorker, /interceptHttps|allowedHosts/, "no host is allowed out by name");
});

test("the question limit is bound and small enough to bound the bill", () => {
  const [limit] = wrangler.ratelimits;
  assert.equal(wrangler.ratelimits.length, 1);
  assert.equal(limit.name, JEV_LIMIT_BINDING);
  assert.match(limit.namespace_id, /^\d+$/);
  assert.ok([10, 60].includes(limit.simple.period));
  // The Worker's cap is the server's own cap on a request to the provider.
  const server = read("../../../crates/maidan-server/src/land_gate_advisor.rs");
  const cap = /pub const MAX_REQUEST_BYTES: usize = (\d+) \* 1024;/.exec(server);
  assert.ok(cap, "the server states its request cap");
  assert.equal(JEV_MAX_BODY_BYTES, Number(cap[1]) * 1024);
  // A day at the configured rate with every question the largest the Worker
  // forwards and a token for every byte of it (no tokenizer charges more), at
  // $0.042 a million input tokens. The limiter is approximate, so this holds
  // the configuration to a design figure; it is not a spending cap.
  const perDay = (limit.simple.limit * 86_400) / limit.simple.period;
  const dollars = (perDay * JEV_MAX_BODY_BYTES * 0.042) / 1_000_000;
  assert.ok(dollars < 30, `a day at the limit would cost at most $${dollars.toFixed(2)}`);
  // Room for a handful of visitors at once: each run of the page asks once,
  // and a visitor who tries every model asks once more for each. At ten a
  // minute, three of them arriving together were refused.
  assert.ok(limit.simple.limit / (limit.simple.period / 60) >= 30);
  assert.equal(wrangler.vars[JEV_KEY_VAR], undefined, "the key is a secret, never a var");
});

test("each provider the Worker holds a key for is offered, pointed at its own name, and no other", () => {
  assert.deepEqual(adviceProviders({}), []);
  const two = { OPENAI_API_KEY: "sk-fixture", GEMINI_API_KEY: " g-fixture " };
  assert.deepEqual(adviceProviders(two), [
    { id: "openai", name: "OpenAI Decisions" },
    { id: "google", name: "Gemini Decisions" },
  ]);
  assert.equal(jevEnabled(two), true);
  const env = adviceServerEnv(two);
  assert.deepEqual(env, {
    MAIDAN_JEV_LAND_GATE_ENABLED: "1",
    MAIDAN_INTERNAL_EGRESS_HOSTS: "openai-decisions.internal,gemini-decisions.internal",
    MAIDAN_OPENAI_DECISIONS_BASE_URL: "http://openai-decisions.internal/",
    OPENAI_API_KEY: JEV_PLACEHOLDER_KEY,
    MAIDAN_GEMINI_DECISIONS_BASE_URL: "http://gemini-decisions.internal/",
    GEMINI_API_KEY: JEV_PLACEHOLDER_KEY,
  });
  // TypeSafe has no key here: the server is told nothing about it, so it
  // would ask TypeSafe's own address, which the container cannot reach.
  assert.equal(env.MAIDAN_JEV_BASE_URL, undefined);
  assert.equal(env.TYPESAFE_API_KEY, undefined);
  const all = demoEnv(undefined, { ...two, [JEV_KEY_VAR]: KEY, PERPLEXITY_API_KEY: "pplx-fixture" });
  assert.equal(all.MAIDAN_INTERNAL_EGRESS_HOSTS, "jev.internal,openai-decisions.internal,gemini-decisions.internal,perplexity-decisions.internal");
  assert.equal(all.MAIDAN_PERPLEXITY_DECISIONS_BASE_URL, "http://perplexity-decisions.internal/");
  assert.equal(all.PERPLEXITY_API_KEY, JEV_PLACEHOLDER_KEY);
  assert.ok(!JSON.stringify(all).match(/sk-fixture|g-fixture|ts_live_fixture|pplx-fixture/), "no real key reaches the container");
  // A Cloudflare token on the Worker turns nothing on in the demo.
  assert.deepEqual(adviceProviders({ CLOUDFLARE_WORKERS_AI_TOKEN: "cf", CLOUDFLARE_ACCOUNT_ID: "0".repeat(32), CLOUDFLARE_API_TOKEN: "cf" }), []);
  // The names the server is given are names it reads (crates/maidan-env).
  const registry = read("../../../crates/maidan-env/src/lib.rs");
  for (const name of Object.keys(all).filter((n) => n.startsWith("MAIDAN_"))) assert.ok(registry.includes(`"${name}"`), name);
});

test("each provider's question goes to its own address with its own key in its own header", async () => {
  const [, openai, google, perplexity] = RELAYS;
  const env = { OPENAI_API_KEY: "sk-fixture", GEMINI_API_KEY: "g-fixture", PERPLEXITY_API_KEY: "pplx-fixture" };
  const up = upstream();
  const o = await forwardAdvice(openai, new Request("http://openai-decisions.internal/v1/decisions", { method: "POST", body: question, headers: { authorization: `Bearer ${JEV_PLACEHOLDER_KEY}` } }), env, up.fn);
  assert.equal(o.status, 200);
  assert.equal(up.calls[0].url, "https://api.openai.com/v1/decisions");
  assert.equal(up.calls[0].init.headers.authorization, "Bearer sk-fixture");
  const g = await forwardAdvice(google, new Request("http://gemini-decisions.internal/v1beta/decisions", { method: "POST", body: question, headers: { "x-goog-api-key": JEV_PLACEHOLDER_KEY } }), env, up.fn);
  assert.equal(g.status, 200);
  assert.equal(up.calls[1].url, "https://generativelanguage.googleapis.com/v1beta/decisions");
  assert.equal(up.calls[1].init.headers["x-goog-api-key"], "g-fixture");
  assert.equal(up.calls[1].init.headers.authorization, undefined);
  const px = await forwardAdvice(perplexity, new Request("http://perplexity-decisions.internal/v1/decisions", { method: "POST", body: question, headers: { authorization: `Bearer ${JEV_PLACEHOLDER_KEY}` } }), env, up.fn);
  assert.equal(px.status, 200);
  assert.equal(up.calls[2].url, "https://api.perplexity.ai/v1/decisions");
  assert.equal(up.calls[2].init.headers.authorization, "Bearer pplx-fixture");
  // Each relay takes its own two paths and no other provider's.
  assert.equal((await forwardAdvice(openai, new Request("http://openai-decisions.internal/v1/systemone", { method: "POST", body: question }), env, up.fn)).status, 404);
  assert.equal((await forwardAdvice(google, new Request("http://gemini-decisions.internal/v1/decisions", { method: "POST", body: question }), env, up.fn)).status, 404);
  assert.equal((await forwardAdvice(google, new Request("http://gemini-decisions.internal/v1beta/models"), env, up.fn)).status, 200);
  // A provider with no key on the Worker is not asked.
  assert.equal((await forwardJev(ask(), env, up.fn)).status, 503);
  assert.equal(up.calls.length, 4);
});
