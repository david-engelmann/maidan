import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { problemPage } from "../src/edge.js";

const read = (name) => readFileSync(new URL(`../site/${name}`, import.meta.url), "utf8");
const pages = ["not-found.html", "unavailable.html", "privacy.html", "problems.html", "support.html", "terms.html"];

test("hosted pages use the warm paper chrome and a focusable skip target", () => {
  const css = readFileSync(new URL("../site/site.css", import.meta.url), "utf8");
  assert.match(css, /--faint:\s*#59554d/);
  assert.match(css, /tr\[id\], h2\[id\] \{ scroll-margin-top: calc\(var\(--safe-top\) \+ 4\.5rem\); \}/);
  assert.match(css, /main:focus:not\(:focus-visible\) \{ outline: none; \}/);
  assert.match(css, /\.small \{ color: var\(--faint\)/);
  for (const name of pages) {
    const html = read(name);
    assert.match(html, /name="color-scheme" content="light"/, name);
    assert.match(html, /name="theme-color" content="#f2eee6"/, name);
    assert.match(html, /<main id="content" tabindex="-1">/, name);
    assert.doesNotMatch(html, /content="dark"|#07080b|<style|style=/, name);
  }
  const problem = problemPage({ title: "Forbidden", detail: "no" }, "req-1");
  assert.match(problem, /name="color-scheme" content="light"/);
  assert.match(problem, /name="theme-color" content="#f2eee6"/);
  assert.match(problem, /<main id="content" tabindex="-1">/);
  assert.match(problem, /class="small"/);
});
