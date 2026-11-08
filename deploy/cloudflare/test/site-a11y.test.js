import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { problemPage } from "../src/edge.js";

const read = (name) => readFileSync(new URL(`../site/${name}`, import.meta.url), "utf8");
const pages = ["not-found.html", "unavailable.html", "privacy.html", "problems.html", "support.html", "terms.html"];

test("hosted pages are dark by default, keep the warm paper for light, and have a focusable skip target", () => {
  const css = readFileSync(new URL("../site/site.css", import.meta.url), "utf8");
  // Dark is the console's default; with no script a page cannot read a
  // stored choice, so the dark palette is the one on :root.
  assert.match(css, /:root, :root\[data-theme="dark"\] \{\s*color-scheme: dark;\s*--bg: #0c0d0f;/);
  assert.match(css, /:root\[data-theme="light"\] \{\s*color-scheme: light;\s*--bg: #f2eee6;/);
  assert.match(css, /--faint:\s*#59554d/);
  // Printers drop backgrounds; light ink on no background would vanish.
  assert.match(css, /@media print \{\s*:root, :root\[data-theme\] \{\s*color-scheme: light;/);
  assert.match(css, /tr\[id\], h2\[id\] \{ scroll-margin-top: calc\(var\(--safe-top\) \+ 4\.5rem\); \}/);
  assert.match(css, /main:focus:not\(:focus-visible\) \{ outline: none; \}/);
  assert.match(css, /\.small \{ color: var\(--faint\)/);
  for (const name of pages) {
    const html = read(name);
    assert.match(html, /name="color-scheme" content="dark"/, name);
    assert.match(html, /name="theme-color" content="#0c0d0f"/, name);
    assert.match(html, /<main id="content" tabindex="-1">/, name);
    assert.doesNotMatch(html, /<style|style=|<script/, name);
  }
  const problem = problemPage({ title: "Forbidden", detail: "no" }, "req-1");
  assert.match(problem, /name="color-scheme" content="dark"/);
  assert.match(problem, /name="theme-color" content="#0c0d0f"/);
  assert.match(problem, /<main id="content" tabindex="-1">/);
  assert.match(problem, /class="small"/);
});
