// The cache helpers against the shared cases in sdk/cache-fixtures/, which the
// other SDKs read too (no server needed).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  CacheError,
  bootPrefix,
  cacheKey,
  cacheKeyFields,
  cachedPrefix,
  gatewaySession,
} from "./index.js";

const CASES = JSON.parse(
  readFileSync(new URL("../cache-fixtures/cases.json", import.meta.url), "utf8"),
);

function expectError(fn, needle) {
  assert.throws(fn, (err) => err instanceof CacheError && err.message.includes(needle));
}

test("the boot prefix keeps the served bytes and hashes them", async () => {
  const got = await bootPrefix(new TextEncoder().encode(CASES.boot.text));
  assert.deepEqual(got, CASES.boot);
  assert.deepEqual(await bootPrefix(CASES.boot.text), CASES.boot);
});

test("the prefix is placed with each provider's breakpoint", () => {
  for (const c of CASES.cached_prefix) {
    const opts = c.ttl === undefined ? {} : { ttl: c.ttl };
    if (c.expected_error) expectError(() => cachedPrefix(c.provider, c.text, opts), c.expected_error);
    else assert.deepEqual(cachedPrefix(c.provider, c.text, opts), c.expected, c.provider);
  }
});

test("a cache key is per workspace and group", async () => {
  for (const c of CASES.cache_key) {
    assert.equal(await cacheKey(c.workspace_id, c.group), c.expected);
  }
  const [a, b] = CASES.cache_key;
  assert.equal(a.group, b.group);
  assert.notEqual(a.expected, b.expected, "two workspaces never share a key");
  await assert.rejects(() => cacheKey("", "g"), CacheError);
});

test("the cache key goes where each provider reads it", () => {
  for (const c of CASES.cache_key_fields) {
    if (c.expected_error) expectError(() => cacheKeyFields(c.provider, "K"), c.expected_error);
    else assert.deepEqual(cacheKeyFields(c.provider, "K"), c.expected, c.provider);
  }
});

test("the thread id is each gateway's session id", () => {
  for (const c of CASES.gateway_session) {
    if (c.expected_error) expectError(() => gatewaySession(c.gateway, c.thread_id), c.expected_error);
    else assert.deepEqual(gatewaySession(c.gateway, c.thread_id), c.expected, c.gateway);
  }
});
