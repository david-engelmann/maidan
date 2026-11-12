// The SDK method map (../methods.json) against this client, offline: every
// mapped method exists, and every example sends the request the map names.
// scripts/check-sdk-parity.py runs this file in CI's lint job.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Client } from "./index.js";

const { methods } = JSON.parse(readFileSync(new URL("../methods.json", import.meta.url), "utf8"));

const camel = (key) => key.replace(/_([a-z])/g, (_, c) => c.toUpperCase());

function resolve(client, name) {
  let owner = client;
  let fn = client;
  for (const part of name.split(".")) {
    owner = fn;
    fn = fn == null ? undefined : fn[part];
  }
  return typeof fn === "function" ? fn.bind(owner) : undefined;
}

test("every mapped method exists", () => {
  const c = new Client("http://sdk.invalid", "t");
  const missing = methods.filter((m) => !resolve(c, m.typescript)).map((m) => `${m.op} (${m.typescript})`);
  assert.deepEqual(missing, []);
});

for (const m of methods.filter((m) => m.example)) {
  test(`${m.op} sends the mapped request`, async () => {
    const ex = m.example;
    const seen = [];
    const fetch = async (url, init) => {
      seen.push({
        method: init.method,
        path: url.slice("http://sdk.invalid".length),
        body: init.body === undefined ? null : JSON.parse(init.body),
      });
      const body = ex.reply.body === null ? null : JSON.stringify(ex.reply.body);
      return new Response(ex.reply.status === 204 ? null : body, {
        status: ex.reply.status,
        headers: { "content-type": "application/json" },
      });
    };
    const c = new Client("http://sdk.invalid", "t", { fetch, maxRetries: 0 });
    const args = [...ex.args];
    if (ex.options) {
      args.push(Object.fromEntries(Object.entries(ex.options).map(([k, v]) => [camel(k), v])));
    }
    const got = await resolve(c, m.typescript)(...args);
    assert.deepEqual(seen, [ex.request]);
    if (ex.reply.body !== null) assert.deepEqual(got, ex.reply.body);
  });
}
