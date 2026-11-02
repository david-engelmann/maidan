import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { edgeRedirect, isPageNavigation, isSitePage, NOT_FOUND_PAGE, PROBLEM_TYPES } from "../src/edge.js";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

// The server's own list, so a problem type added there without a row here
// fails.
const serverTypes = [
  ...read("../../../crates/maidan-server/src/error.rs").matchAll(/"https:\/\/maidan\.world\/problems\/([a-z-]+)"/g),
].map((m) => m[1]);
const pageRows = [...read("../site/problems.html").matchAll(/<tr id="([a-z-]+)">/g)].map((m) => m[1]);

test("every problem type the server sends has a row on the problems page, and no other", () => {
  const sent = new Set(serverTypes);
  assert.ok(sent.size >= 20, `read ${sent.size} problem types from error.rs`);
  assert.deepEqual([...PROBLEM_TYPES].sort(), [...sent].sort());
  assert.deepEqual([...pageRows].sort(), [...sent].sort());
});

test("a problem type's URI lands on its row", () => {
  const res = edgeRedirect(new Request("https://maidan.world/problems/cursor-too-old"), { hasSite: true });
  assert.equal(res.status, 302);
  assert.equal(res.headers.get("location"), "https://maidan.world/problems#cursor-too-old");
});

test("an unknown problem name, and an instance without a site, reach the container", () => {
  assert.equal(edgeRedirect(new Request("https://maidan.world/problems/no-such-type"), { hasSite: true }), null);
  assert.equal(edgeRedirect(new Request("https://dev.maidan.world/problems/not-found")), null);
});

test("the problems page and the not-found page are site pages", () => {
  assert.ok(isSitePage(new Request("https://maidan.world/problems")));
  assert.match(read(`../site${NOT_FOUND_PAGE}.html`), /<title>Not found · Maidan<\/title>/);
});

test("only a browser opening a page counts as a page navigation", () => {
  const nav = (headers, method = "GET") => isPageNavigation(new Request("https://maidan.world/x", { method, headers }));
  assert.ok(nav({ "sec-fetch-dest": "document", accept: "text/html" }));
  assert.ok(nav({ accept: "text/html,application/xhtml+xml" }));
  assert.ok(!nav({ "sec-fetch-dest": "empty", accept: "text/html" }));
  assert.ok(!nav({ accept: "application/json" }));
  assert.ok(!nav({}));
  assert.ok(!nav({ accept: "text/html" }, "POST"));
});
