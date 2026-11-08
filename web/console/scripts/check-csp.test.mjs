import { test } from "node:test";
import assert from "node:assert/strict";
import { cspViolations, missingPreloads, themeLoadsLate } from "./check-csp.mjs";

test("an external module script and stylesheet pass", () => {
  const html = '<link rel="stylesheet" href="/ui/style.css"><script type="module" crossorigin src="/ui/app.js"></script>';
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

test("the theme script must load, plainly, before the stylesheet", () => {
  const sheet = '<link rel="stylesheet" href="/ui/style.css">';
  const boot = '<script src="/ui/theme.js"></script>';
  assert.equal(themeLoadsLate(boot + sheet, "/ui/"), null);
  assert.match(themeLoadsLate(sheet + boot, "/ui/") ?? "", /stylesheet before/);
  assert.match(themeLoadsLate(sheet, "/ui/") ?? "", /does not load/);
  assert.match(themeLoadsLate('<script type="module" src="/ui/theme.js"></script>' + sheet, "/ui/") ?? "", /does not load/);
});
