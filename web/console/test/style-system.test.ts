import { test } from "node:test";
import assert from "node:assert/strict";
import stylelint from "stylelint";
import config from "../stylelint.config.js";
import { sheet } from "./sheet.ts";

// The console's spacing, type and breakpoints are tokens (UI-003, UI-004).
// stylelint holds the line in `npm run lint`; these pin what it refuses, so a
// loosened rule fails here as well as letting a raw value through.

async function problems(code: string): Promise<string[]> {
  const { results } = await stylelint.lint({ code, config });
  return results.flatMap((r) => r.warnings.map((w) => w.rule));
}

test("a padding, margin or gap takes a space token, not a raw px", async () => {
  for (const code of [".a { padding: 8px; }", ".a { margin: 0 12px; }", ".a { gap: 10px; }", ".a { padding-inline: 14px; }", ".a { margin: max(8px, 1rem) 0; }"]) {
    assert.deepEqual(await problems(code), ["declaration-property-value-disallowed-list"], code);
  }
  for (const code of [".a { padding: var(--space-2) var(--space-3); }", ".a { padding: 1px 7px; }", ".a { margin-left: -4px; }", ".a { gap: 0; }", ".a { padding: 1rem var(--pad); }"]) {
    assert.deepEqual(await problems(code), [], code);
  }
});

test("a font size takes a type token", async () => {
  assert.deepEqual(await problems(".a { font-size: 18px; }"), ["declaration-property-value-disallowed-list"]);
  assert.deepEqual(await problems(".a { font-size: var(--text-title); }"), []);
});

test("a media query uses a named breakpoint", async () => {
  assert.deepEqual(await problems("@media (max-width: 720px) { .a { color: red; } }"), ["media-feature-name-value-allowed-list"]);
  for (const w of ["(max-width: 380px)", "(max-width: 599px)", "(min-width: 600px)", "(max-width: 899px)", "(min-width: 900px)", "(min-width: 1440px)"]) {
    assert.deepEqual(await problems(`@media ${w} { .a { color: red; } }`), [], w);
  }
});

test("every space and type token the sheet uses is defined once, in the tokens", () => {
  const css = sheet().replace(/\/\*[\s\S]*?\*\//g, "");
  const used = new Set([...css.matchAll(/var\((--(?:space|text)-[\w-]+)\)/g)].map((m) => m[1]));
  for (const name of used) {
    assert.equal(css.match(new RegExp(`${name}:`, "g"))?.length, 1, `${name} is defined ${css.match(new RegExp(`${name}:`, "g"))?.length ?? 0} times`);
  }
  assert.ok(used.size >= 20, `only ${used.size} tokens used; the scan may be broken`);
});
