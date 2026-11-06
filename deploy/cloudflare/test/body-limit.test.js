import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { HOSTED_MAX_BODY_BYTES, tooLarge } from "../src/body-limit.js";

const indexSrc = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");

test("hosted max body matches the server default of 2 MiB", () => {
  assert.equal(HOSTED_MAX_BODY_BYTES, 2 * 1024 * 1024);
});

test("tooLarge refuses only when Content-Length exceeds the cap", () => {
  const req = (n) => new Request("https://maidan.world/x", { headers: n == null ? {} : { "content-length": String(n) } });
  assert.equal(tooLarge(req(null)), false);
  assert.equal(tooLarge(req(0)), false);
  assert.equal(tooLarge(req(HOSTED_MAX_BODY_BYTES)), false);
  assert.equal(tooLarge(req(HOSTED_MAX_BODY_BYTES + 1)), true);
  assert.equal(tooLarge(req("nope")), false);
});

test("the hosted worker refuses oversized bodies before waking the container", () => {
  const handler = indexSrc.slice(indexSrc.lastIndexOf("async fetch("), indexSrc.indexOf("async scheduled("));
  const gate = handler.indexOf("tooLarge(request)");
  const wake = handler.indexOf("getContainer(env.MAIDAN).fetch(new Request");
  assert.ok(gate > 0 && wake > gate, "tooLarge runs before the request is forwarded to the container");
  assert.match(handler, /status: 413/);
});
