import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// wrangler accepts JSONC; strip whole-line comments, which is all this file uses.
const config = JSON.parse(
  readFileSync(new URL("../wrangler.docs.jsonc", import.meta.url), "utf8")
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n"),
);

test("the docs Worker serves only docs.maidan.world, never workers.dev", () => {
  assert.equal(config.name, "maidan-docs");
  assert.equal(config.workers_dev, false);
  assert.deepEqual(config.routes, [{ pattern: "docs.maidan.world", custom_domain: true }]);
});

test("the docs Worker is static assets from the mdBook HTML build, with no code", () => {
  assert.equal(config.main, undefined);
  assert.equal(config.assets.directory, "../../book/build/html");
  assert.equal(config.assets.not_found_handling, "404-page");
});

test("the docs answers carry HSTS, nosniff, framing denial, and a fail-closed CSP", () => {
  const headers = readFileSync(new URL("../docs-headers", import.meta.url), "utf8");
  assert.match(headers, /^\/\*$/m);
  assert.match(headers, /Strict-Transport-Security: max-age=\d+/);
  assert.match(headers, /X-Content-Type-Options: nosniff/);
  assert.match(headers, /X-Frame-Options: DENY/);
  assert.match(headers, /Cross-Origin-Opener-Policy: same-origin/);
  assert.match(headers, /Cross-Origin-Resource-Policy: same-origin/);
  assert.match(headers, /Permissions-Policy: camera=\(\), microphone=\(\), geolocation=\(\), payment=\(\)/);
  assert.match(headers, /Content-Security-Policy: default-src 'none'/);
  assert.match(headers, /script-src 'self'/);
  assert.match(headers, /frame-ancestors 'none'/);
  assert.doesNotMatch(headers, /unsafe-eval/);
});
