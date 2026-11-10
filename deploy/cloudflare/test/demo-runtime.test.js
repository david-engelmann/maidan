import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { DEMO_UNAVAILABLE_CSP, demoUnavailablePage, fetchDemo } from "../src/demo-edge.js";
import { SECURITY_HEADERS, UNAVAILABLE_RETRY_SECS } from "../src/edge.js";
import { demoEnv } from "../src/demo-env.js";
import { envFingerprint } from "../src/server-env.js";

const demoWorker = readFileSync(new URL("../src/demo.js", import.meta.url), "utf8");
const demoEdge = readFileSync(new URL("../src/demo-edge.js", import.meta.url), "utf8");
// theme.js is copied into the try assets at deploy (deploy-demo.yml), so it
// is not in the checkout's try/ directory.
const tryFiles = new Set([...readdirSync(new URL("../try/", import.meta.url)).map((name) => `/${name}`), "/theme.js"]);

const req = (path, headers = {}) => new Request(`https://try.maidan.world${path}`, { headers });
const plain = (status, body) =>
  new Response(body, { status, headers: { "content-type": "text/plain;charset=UTF-8" } });

function silenceConsole() {
  mock.method(console, "error", () => {});
  mock.method(console, "log", () => {});
}

test("the demo class fingerprints its own environment and recycles through the session", () => {
  assert.match(demoWorker, /this\.envVars = demoEnv\(env\[DEMO_PUBLIC_ORIGIN_VAR\]\)/);
  assert.match(demoWorker, /this\.envSession = new ContainerEnvSession\(\)/);
  const fetch = demoWorker.slice(demoWorker.indexOf("async fetch(request)"), demoWorker.indexOf("async reset()"));
  assert.match(fetch, /envFingerprint\(this\.envVars\)/);
  assert.match(fetch, /this\.envSession\.serve\(/);
  assert.match(fetch, /forward: \(\) => super\.fetch\(request\)/);
  assert.match(fetch, /blockConcurrencyWhile/);
  assert.doesNotMatch(fetch, /serverEnv\(/);
});

test("the daily reset invalidates the session under its lock and only destroys a running container", () => {
  const reset = demoWorker.slice(demoWorker.indexOf("async reset()"), demoWorker.indexOf("export default"));
  assert.match(reset, /blockConcurrencyWhile/);
  assert.match(
    reset,
    /invalidate\(\)[\s\S]*if \(this\.ctx\.container\.running\) await this\.destroy\(\);/,
  );
  assert.doesNotMatch(reset, /\.(fetch|start|startAndWaitForPorts|containerFetch)\(/);
  assert.match(demoWorker, /sleepAfter = "15m";/);
});

test("the worker wraps the container fetch, and the try page still comes first", () => {
  const handler = demoWorker.slice(demoWorker.lastIndexOf("async fetch("), demoWorker.indexOf("async scheduled("));
  const own = handler.indexOf("handleTry(");
  const container = handler.indexOf("getContainer(");
  assert.ok(own > 0 && container > own, "handleTry runs before getContainer");
  assert.match(handler, /return fetchDemo\(request, \(req\) => getContainer\(env\.MAIDAN_DEMO\)\.fetch\(req\)\)/);
  assert.match(demoEdge, /try \{[\s\S]*fetchContainer\(/);
  assert.match(demoEdge, /catch \(err\)/);
  assert.match(demoEdge, /isContainerUnavailable\(response\)/);
});

test("a changed demo origin is a different fingerprint", async () => {
  const live = await envFingerprint(demoEnv("https://try.maidan.world"));
  const other = await envFingerprint(demoEnv("https://preview.example"));
  assert.notEqual(live, other);
  assert.equal(live, await envFingerprint(demoEnv(undefined)));
});

test("a thrown container fetch becomes a 503 problem with Retry-After, and the error text stays out", async () => {
  silenceConsole();
  const res = await fetchDemo(req("/sandbox", { accept: "application/json" }), async () => {
    throw new Error("boom-secret container failed");
  });
  assert.equal(res.status, 503);
  assert.equal(res.headers.get("content-type"), "application/problem+json");
  assert.equal(res.headers.get("retry-after"), String(UNAVAILABLE_RETRY_SECS));
  assert.equal(res.headers.get("cache-control"), "no-store");
  assert.equal(res.headers.get("strict-transport-security"), SECURITY_HEADERS["strict-transport-security"]);
  assert.equal(res.headers.get("x-content-type-options"), "nosniff");
  const body = await res.json();
  assert.equal(body.type, "https://maidan.world/problems/overloaded");
  assert.equal(body.status, 503);
  assert.match(body.detail, /starting or being updated/);
  assert.doesNotMatch(JSON.stringify(body), /boom-secret/);
});

test("the runtime's no-container page is a 503, and the server's own 503 passes through", async () => {
  silenceConsole();
  const down = await fetchDemo(req("/mcp", { accept: "application/json" }), async () =>
    plain(500, "Error proxying request to container: The container is not running"),
  );
  assert.equal(down.status, 503);
  assert.equal(down.headers.get("retry-after"), String(UNAVAILABLE_RETRY_SECS));
  assert.equal((await down.json()).type, "https://maidan.world/problems/overloaded");

  const cap = new Response(
    JSON.stringify({
      type: "https://maidan.world/problems/overloaded",
      title: "Service Unavailable",
      status: 503,
      detail: "the demo is full until its daily reset; try again later",
    }),
    { status: 503, headers: { "content-type": "application/problem+json" } },
  );
  const passed = await fetchDemo(req("/sandbox", { accept: "application/json" }), async () => cap);
  assert.equal(passed.status, 503);
  assert.equal(passed.headers.get("content-type"), "application/problem+json");
  assert.equal(passed.headers.get("retry-after"), null, "the daily cap has no Retry-After");
  assert.match(await passed.text(), /full until its daily reset/);
  assert.equal(passed.headers.get("x-content-type-options"), "nosniff");

  const other = await fetchDemo(req("/mcp"), async () => plain(500, "something else"));
  assert.equal(other.status, 500);
  assert.match(await other.text(), /something else/);
});

test("a browser opening a page while the container is down gets HTML that only uses the try assets", async () => {
  silenceConsole();
  const nav = req("/ui/", { "sec-fetch-dest": "document", accept: "text/html" });
  const res = await fetchDemo(nav, async () => {
    throw new Error("not running");
  });
  assert.equal(res.status, 503);
  assert.match(res.headers.get("content-type"), /^text\/html/);
  assert.equal(res.headers.get("content-security-policy"), DEMO_UNAVAILABLE_CSP);
  assert.equal(res.headers.get("retry-after"), String(UNAVAILABLE_RETRY_SECS));
  assert.equal(res.headers.get("cache-control"), "no-store");
  assert.doesNotMatch(DEMO_UNAVAILABLE_CSP, /unsafe-inline|unsafe-eval/);
  const html = await res.text();
  assert.match(html, new RegExp(`<meta http-equiv="refresh" content="${UNAVAILABLE_RETRY_SECS}">`));
  assert.match(html, /The demo is starting or being updated/);
  // One script, the console's pre-paint theme.js, served beside the page.
  assert.deepEqual([...html.matchAll(/<script[^>]*>/g)].map((m) => m[0]), ['<script src="/theme.js">']);
  assert.match(DEMO_UNAVAILABLE_CSP, /script-src 'self';/);
  assert.doesNotMatch(html, /<style|\sstyle=/);
  assert.doesNotMatch(html, /site\.css|\/status|\/support|\/privacy/);
  assert.doesNotMatch(html, /not running/);
  for (const ref of [...html.matchAll(/(?:href|src)="(\/[^"]+)"/g)].map((m) => m[1])) {
    if (ref === "/") continue;
    assert.ok(tryFiles.has(ref), `${ref} is not a file the demo serves`);
  }
  assert.ok(tryFiles.has("/try.css"));
  assert.equal(tryFiles.has("/site.css"), false);

  const headRes = await fetchDemo(
    new Request("https://try.maidan.world/ui/", { method: "HEAD", headers: { "sec-fetch-dest": "document" } }),
    async () => plain(500, "Error proxying request to container"),
  );
  assert.equal(headRes.status, 503);
  assert.match(headRes.headers.get("content-type"), /^text\/html/);
});

test("an API client and a script are not handed the HTML page", async () => {
  silenceConsole();
  const api = await fetchDemo(
    req("/sandbox", { accept: "application/json", "sec-fetch-dest": "empty" }),
    async () => plain(500, "Error proxying request to container"),
  );
  assert.equal(api.headers.get("content-type"), "application/problem+json");
  const post = await fetchDemo(
    new Request("https://try.maidan.world/sandbox", {
      method: "POST",
      headers: { accept: "text/html", "sec-fetch-dest": "document" },
    }),
    async () => {
      throw new Error("down");
    },
  );
  assert.equal(post.headers.get("content-type"), "application/problem+json");
});

test("a WebSocket upgrade passes through untouched", async () => {
  const upgrade = { status: 101, headers: new Headers({ upgrade: "websocket" }), webSocket: {} };
  const res = await fetchDemo(req("/ws", { upgrade: "websocket" }), async () => upgrade);
  assert.equal(res, upgrade);
  assert.equal(res.status, 101);
});

test("the container is told the visitor's address and that the hop is HTTPS", async () => {
  let seen;
  const res = await fetchDemo(
    req("/health", { "cf-connecting-ip": "203.0.113.8", "x-forwarded-for": "10.0.0.1" }),
    async (forwarded) => {
      seen = forwarded;
      return new Response("ok");
    },
  );
  assert.equal(res.status, 200);
  assert.equal(seen.headers.get("x-forwarded-for"), "203.0.113.8");
  assert.equal(seen.headers.get("x-forwarded-proto"), "https");
  assert.equal(res.headers.get("strict-transport-security"), SECURITY_HEADERS["strict-transport-security"]);
});

test("the unavailable page is the same document the navigation fallback sends", async () => {
  const page = demoUnavailablePage();
  assert.equal(page.status, 503);
  assert.equal(page.headers.get("retry-after"), String(UNAVAILABLE_RETRY_SECS));
  const html = await page.text();
  assert.match(html, /href="\/try\.css"/);
  assert.doesNotMatch(html, /site\.css/);
});
