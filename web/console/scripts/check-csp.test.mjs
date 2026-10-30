import { test } from "node:test";
import assert from "node:assert/strict";
import { cspViolations } from "./check-csp.mjs";

test("an external module script and stylesheet pass", () => {
  const html = '<link rel="stylesheet" href="/console/style.css"><script type="module" crossorigin src="/console/app.js"></script>';
  assert.deepEqual(cspViolations(html), []);
});

test("an inline script fails", () => {
  assert.deepEqual(cspViolations("<script>alert(1)</script>"), ["inline <script> without src"]);
});

test("a style element fails", () => {
  assert.deepEqual(cspViolations("<style>body{}</style>"), ["<style> element"]);
});

test("an inline handler or javascript: URL fails", () => {
  assert.deepEqual(cspViolations('<a href="javascript:x" onclick="y">'), [
    "inline event handler attribute",
    "javascript: URL",
  ]);
});
