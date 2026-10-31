import { test } from "node:test";
import assert from "node:assert/strict";
import { cspViolations, missingPreloads } from "./check-csp.mjs";

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

test("every chunk the entry imports must be preloaded", () => {
  const entry = 'import{r as l}from"./vendor.js";import"./extra.js";';
  const html = '<script type="module" src="/x/app.js"></script><link rel="modulepreload" crossorigin href="/x/vendor.js">';
  assert.deepEqual(missingPreloads(html, entry, "/x/"), ["extra.js"]);
  assert.deepEqual(missingPreloads(html + '<link rel="modulepreload" href="/x/extra.js">', entry, "/x/"), []);
});
