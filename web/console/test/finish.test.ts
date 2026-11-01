import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { TOKEN_PLACEHOLDER, cursorDeeplink, mcpUrl, snippets } from "../src/core/connect.ts";
import { parseResult, transitionsFor, type Thread } from "../src/core/board.ts";
import { pushRefusal } from "../src/pwa/push.ts";
import { SHELL_FILES } from "../src/pwa/policy.ts";

const b64 = (s: string) => Buffer.from(s).toString("base64");

test("connect snippets point at the MCP endpoint and never carry a token", () => {
  assert.equal(mcpUrl("https://m.example/"), "https://m.example/mcp/streamable");
  const all = snippets("https://m.example").map((s) => s.text).join("\n");
  assert.match(all, /https:\/\/m\.example\/mcp\/streamable/);
  assert.ok(all.includes(TOKEN_PLACEHOLDER));
  const link = cursorDeeplink("https://m.example", b64);
  const cfg = JSON.parse(Buffer.from(decodeURIComponent(link.split("config=")[1]), "base64").toString());
  assert.equal(cfg.url, "https://m.example/mcp/streamable");
  assert.equal(cfg.headers.Authorization, `Bearer ${TOKEN_PLACEHOLDER}`);
});

test("transitions offered follow the state machine", () => {
  const th = (state: string) => ({ id: "t", state }) as Thread;
  assert.deepEqual(transitionsFor(th("open")), ["start_review", "archive"]);
  assert.deepEqual(transitionsFor(th("in_review")), ["archive"]);
  assert.deepEqual(transitionsFor(th("closed")), ["archive"]);
  assert.deepEqual(transitionsFor(th("archived")), []);
});

test("a typed result is JSON when it parses, else a summary", () => {
  assert.deepEqual(parseResult('{"ok":true}'), { ok: true, value: { ok: true } });
  assert.deepEqual(parseResult("shipped it"), { ok: true, value: { summary: "shipped it" } });
  assert.equal(parseResult("  ").ok, false);
});

test("Turn on explains itself when it cannot run", () => {
  assert.equal(pushRefusal({ me: "m", permission: "default", hasKey: true }), null);
  assert.equal(pushRefusal({ me: "m", permission: "default", hasKey: null }), null);
  assert.match(pushRefusal({ me: "m", permission: "denied", hasKey: true }) ?? "", /site settings/);
  assert.match(pushRefusal({ me: "m", permission: "granted", hasKey: false }) ?? "", /no push key/);
  assert.match(pushRefusal({ me: null, permission: "granted", hasKey: true }) ?? "", /Sign in/);
});

test("the manifest lists 192 and 512 icons and the worker caches them", () => {
  const m = JSON.parse(readFileSync(new URL("../public/manifest.webmanifest", import.meta.url), "utf8"));
  const sizes = m.icons.map((i: { sizes: string }) => i.sizes);
  assert.ok(sizes.includes("192x192") && sizes.includes("512x512"));
  assert.ok((SHELL_FILES as readonly string[]).includes("icon-512.png"));
});
