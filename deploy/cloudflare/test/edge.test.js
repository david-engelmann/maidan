import { test } from "node:test";
import assert from "node:assert/strict";
import { edgeRedirect, withSecurityHeaders, SECURITY_HEADERS } from "../src/edge.js";

const get = (url, accept) =>
  new Request(url, { headers: accept ? { accept } : {} });

test("plain HTTP is sent to HTTPS with its path and query", () => {
  const res = edgeRedirect(get("http://maidan.world/ui/?x=1"));
  assert.equal(res.status, 301);
  assert.equal(res.headers.get("location"), "https://maidan.world/ui/?x=1");
});

test("a browser opening the bare origin lands in the console", () => {
  const res = edgeRedirect(get("https://maidan.world/", "text/html,application/xhtml+xml"));
  assert.equal(res.status, 302);
  assert.equal(res.headers.get("location"), "https://maidan.world/ui/");
});

test("an API client, any other path, and a POST reach the container", () => {
  assert.equal(edgeRedirect(get("https://maidan.world/", "application/json")), null);
  assert.equal(edgeRedirect(get("https://maidan.world/health", "text/html")), null);
  const post = new Request("https://maidan.world/", { method: "POST", headers: { accept: "text/html" } });
  assert.equal(edgeRedirect(post), null);
});

test("every answer gains the security headers it did not set", () => {
  const res = withSecurityHeaders(new Response("ok", { headers: { "referrer-policy": "no-referrer" } }));
  assert.equal(res.headers.get("strict-transport-security"), SECURITY_HEADERS["strict-transport-security"]);
  assert.equal(res.headers.get("x-content-type-options"), "nosniff");
  assert.equal(res.headers.get("referrer-policy"), "no-referrer", "the server's own value wins");
});

test("a WebSocket upgrade passes through untouched", () => {
  const upgrade = { status: 101, headers: new Headers() };
  assert.equal(withSecurityHeaders(upgrade), upgrade);
});
