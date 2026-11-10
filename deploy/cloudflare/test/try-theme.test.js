import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { TRY_PATHS } from "../src/try-page.js";
import { demoUnavailablePage, DEMO_UNAVAILABLE_CSP } from "../src/demo-edge.js";
import { themeLoadsLate } from "../../../web/console/scripts/check-csp.mjs";

// try.maidan.world wears the rest of the product's theme: dark by default,
// the console's stored choice honoured before the first paint, the landing's
// tokens, and AA contrast in both palettes. It was once light only, and
// nothing held it to the console's own checks (web/console/test
// tokens.test.ts and contrast.test.ts), which these mirror.

const read = (rel) => readFileSync(new URL(rel, import.meta.url), "utf8");
const strip = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const css = strip(read("../try/try.css"));
const landing = strip(read("../../../web/landing/src/styles.css"));
const page = read("../try/index.html");

const HEADS = { dark: ':root, :root[data-theme="dark"] {', light: ':root[data-theme="light"] {' };

/** The body of the one palette block for a theme. */
function palette(source, scheme) {
  const at = source.indexOf(HEADS[scheme]);
  assert.ok(at >= 0, `${scheme} palette block`);
  return source.slice(at + HEADS[scheme].length, source.indexOf("}", at));
}

function tokens(source, scheme) {
  return new Map([...palette(source, scheme).matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

function hex(t, name) {
  let v = t.get(name);
  for (let k = 0; v && v.startsWith("var(") && k < 5; k++) v = t.get(v.slice(4, -1).trim());
  assert.match(v ?? "", /^#[0-9a-f]{6}$/i, `${name} resolves to a hex colour`);
  return v;
}

function ratio(a, b) {
  const lum = (h) => {
    const [r, g, bl] = [1, 3, 5]
      .map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
      .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

const SURFACES = ["--paper", "--paper-2", "--card"];

function short(scheme, fgs, bgs, min) {
  const t = tokens(css, scheme);
  return fgs.flatMap((fg) =>
    bgs
      .map((bg) => [fg, bg, ratio(hex(t, fg), hex(t, bg))])
      .filter(([, , r]) => r < min)
      .map(([f, b, r]) => `${f} on ${b} is ${r.toFixed(2)}:1`),
  );
}

test("the try page is dark by default, in the console's graphite and orange", () => {
  assert.match(css, /:focus-visible \{ outline: 2px solid var\(--focus\);/);
  const dark = tokens(css, "dark");
  assert.match(palette(css, "dark"), /color-scheme: dark;/);
  assert.match(palette(css, "light"), /color-scheme: light;/);
  assert.equal(dark.get("--paper"), "#0c0d0f");
  assert.equal(dark.get("--paper-2"), "#16171b");
  assert.equal(dark.get("--card"), "#1c1e22");
  assert.equal(dark.get("--signal"), "#ff6a2b");
  assert.match(page, /<meta name="color-scheme" content="dark light">/);
  assert.match(page, /<meta name="theme-color" content="#0c0d0f">/);
});

test("each theme sets its palette in one block, and the system choice is theme.js's, not a media query's", () => {
  assert.equal(css.split(HEADS.dark).length - 1, 1);
  assert.equal(css.split(HEADS.light).length - 1, 1);
  assert.doesNotMatch(css, /prefers-color-scheme/);
});

test("both palettes set the same tokens, so neither theme falls back to the other's colour", () => {
  const names = (scheme) => [...tokens(css, scheme).keys()].sort();
  assert.deepEqual(names("light"), names("dark"));
});

test("every token the try page shares with the landing has the landing's value", () => {
  for (const scheme of ["dark", "light"]) {
    const mine = tokens(css, scheme);
    const theirs = tokens(landing, scheme);
    const drift = [...mine]
      .filter(([name, value]) => theirs.has(name) && theirs.get(name) !== value)
      .map(([name, value]) => `${scheme} ${name}: ${value}, the landing has ${theirs.get(name)}`);
    assert.deepEqual(drift, []);
  }
});

test("every custom property a rule reads is defined", () => {
  const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
  const missing = [...css.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]).filter((v) => !defined.has(v));
  assert.deepEqual([...new Set(missing)], []);
});

test("no colour is written outside the token blocks", () => {
  const rules = css.replace(/(?<=^|[{};])\s*:root[^{]*\{[^}]*\}/g, "");
  const literals = [...rules.matchAll(/#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(|\b(?:white|black)\b(?!-)/gi)].map(
    (m) => `${m[0]} near "${rules.slice(Math.max(0, m.index - 40), m.index + 20).replace(/\s+/g, " ")}"`,
  );
  assert.deepEqual(literals, []);
});

for (const scheme of ["dark", "light"]) {
  test(`${scheme}: text, muted text, links and errors reach 4.5:1 on every surface`, () => {
    assert.deepEqual(short(scheme, ["--ink", "--muted", "--signal-ink", "--danger"], SURFACES, 4.5), []);
  });

  test(`${scheme}: the primary action and the code blocks keep their text at 4.5:1`, () => {
    assert.deepEqual([...short(scheme, ["--primary-ink"], ["--primary-flat"], 4.5), ...short(scheme, ["--run-ink"], ["--run-bg"], 4.5)], []);
  });

  test(`${scheme}: the focus ring and outlined buttons reach 3:1 on every surface`, () => {
    assert.deepEqual(short(scheme, ["--focus", "--ink"], SURFACES, 3), []);
  });
}

test("dark surfaces step up: page, then the notice, then the card", () => {
  const t = tokens(css, "dark");
  const steps = SURFACES.map((n) => hex(t, n));
  for (let i = 1; i < steps.length; i++) assert.ok(ratio(steps[i], "#000000") > ratio(steps[i - 1], "#000000"), `${steps[i]} is not above ${steps[i - 1]}`);
});

test("the try page and the demo's unavailable page load the console's theme.js before their stylesheet", async () => {
  assert.equal(themeLoadsLate(page, "/"), null);
  assert.equal(themeLoadsLate(await demoUnavailablePage().text(), "/"), null);
  assert.ok(TRY_PATHS.has("/theme.js"));
  assert.match(DEMO_UNAVAILABLE_CSP, /script-src 'self'/);
});

test("theme.js is the console's file, copied in at deploy and never kept as a fork", () => {
  // The Worker serves it from the try assets, so a visit never wakes the
  // container for the console's /ui/theme.js.
  const workflow = read("../../../.github/workflows/deploy-demo.yml");
  const copy = workflow.indexOf("cp ../../web/console/public/theme.js try/theme.js");
  assert.ok(copy > 0, "deploy-demo.yml copies the console's theme.js into the try assets");
  assert.ok(copy < workflow.indexOf("npx wrangler deploy -c wrangler.demo.jsonc"), "the copy comes before the deploy");
  assert.match(read("../../../.gitignore"), /^deploy\/cloudflare\/try\/theme\.js$/m);
  assert.match(read("../../../web/console/public/theme.js"), /maidan_console_theme/);
});
