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
  assert.deepEqual(Object.keys(jevOutboundByHost), [JEV_INTERNAL_HOST]);
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
  // A day at the limit, at 2,000 input tokens a question and $0.042 a million.
  const perDay = (limit.simple.limit * 86_400) / limit.simple.period;
  const dollars = (perDay * 2_000 * 0.042) / 1_000_000;
  assert.ok(dollars < 5, `a day at the limit would cost about $${dollars.toFixed(2)}`);
  assert.equal(wrangler.vars[JEV_KEY_VAR], undefined, "the key is a secret, never a var");
});
