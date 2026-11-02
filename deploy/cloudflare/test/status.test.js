import { test } from "node:test";
import assert from "node:assert/strict";
import { isStatusRequest, statusResponse, summarize, STATUS_CSP } from "../src/status.js";

const req = (path, method = "GET") => new Request(`https://maidan.world${path}`, { method });
const healthy = { status: "ok", db: "ok", storage: "ok", indexer: "ok", bus: "ok", version: "1.2.3" };
const json = (body, status = 200) => async () => new Response(JSON.stringify(body), { status });

test("only GET and HEAD of /status and its stylesheet are the status page", () => {
  assert.equal(isStatusRequest(req("/status")), true);
  assert.equal(isStatusRequest(req("/status.css")), true);
  assert.equal(isStatusRequest(req("/status", "HEAD")), true);
  assert.equal(isStatusRequest(req("/status", "POST")), false);
  assert.equal(isStatusRequest(req("/status/x")), false);
  assert.equal(isStatusRequest(req("/health")), false);
});

test("a healthy instance says all systems are working, with each part", async () => {
  const res = await statusResponse(req("/status"), json(healthy));
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("content-security-policy"), STATUS_CSP);
  assert.equal(res.headers.get("cache-control"), "no-store");
  const html = await res.text();
  assert.match(html, /All systems working\./);
  for (const label of ["Database", "Artifact storage", "Live updates", "Search index"]) assert.match(html, new RegExp(label));
  assert.match(html, /version 1\.2\.3/);
  assert.doesNotMatch(html, /<script|<style|style=/);
});

test("a part in trouble is named, but its error text never shows", async () => {
  const body = { ...healthy, status: "degraded", db: { error: "connect to db.internal.example:5432 refused" } };
  const html = await (await statusResponse(req("/status"), json(body, 503))).text();
  assert.match(html, /Some parts are having trouble\./);
  assert.match(html, /Database<\/span><span class="bad">Trouble/);
  assert.doesNotMatch(html, /internal\.example|refused/);
});

test("a container that does not answer, or answers garbage, is reported plainly", async () => {
  const thrown = await (await statusResponse(req("/status"), async () => { throw new Error("down"); })).text();
  assert.match(thrown, /not answering/);
  const slow = await (await statusResponse(req("/status"), () => new Promise(() => {}), { timeoutMs: 10 })).text();
  assert.match(slow, /not answering/);
  const garbage = await (await statusResponse(req("/status"), async () => new Response("<html>"))).text();
  assert.match(garbage, /not answering/);
});

test("the stylesheet is served as CSS and HEAD has no body", async () => {
  const css = await statusResponse(req("/status.css"), json(healthy));
  assert.match(css.headers.get("content-type"), /text\/css/);
  const head = await statusResponse(req("/status", "HEAD"), json(healthy));
  assert.equal(await head.text(), "");
});

test("summarize drops anything it does not know and escapes the version", () => {
  const s = summarize({ status: "ok", db: "ok", secret: "x", version: "<b>" });
  assert.deepEqual(s.parts, [{ label: "Database", ok: true }]);
  assert.equal(s.version, "<b>");
});
