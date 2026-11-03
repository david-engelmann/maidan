import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// The console's surfaces share one radius scale and one palette, so a corner
// or a colour changes in one place and light and dark stay in step.

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
// Set at runtime from script (viewport, keyboard, composer, drag position, font).
const RUNTIME = new Set(["--vv-top", "--vv-h", "--composer-h", "--kb", "--left", "--font"]);

test("every corner above 4px comes from the radius scale", () => {
  const raw = [...css.matchAll(/border-radius:\s*(\d+)px/g)].map((m) => Number(m[1])).filter((n) => n > 4);
  assert.deepEqual(raw, []);
});

test("every custom property a rule reads is defined or set at runtime", () => {
  const missing = [...css.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]).filter((v) => !defined.has(v) && !RUNTIME.has(v));
  assert.deepEqual([...new Set(missing)], []);
});

test("the review colour is a token with a dark twin, not a fallback literal", () => {
  assert.ok(!/var\(--review,/.test(css));
  const dark = css.slice(css.indexOf("prefers-color-scheme: dark"));
  assert.match(dark, /--review:\s*#/);
});

test("in dark mode a board card sits a step above its lane, and the lane above the page", () => {
  const dark = css.slice(css.lastIndexOf("@media (prefers-color-scheme: dark)"));
  const bg = (sel: string) => dark.match(new RegExp(`${sel.replace(/[.]/g, "\\.")} \\{[^}]*background: var\\((--[\\w-]+)\\)`))?.[1];
  assert.equal(bg(".lanes.wide .lane"), "--elev-1");
  assert.equal(bg(".task"), "--elev-2");
});

test("the board's new-task form and the thread composer do not share a class", () => {
  // They once did, and the board's flex-wrap rule shrank the thread's message field to a pill.
  assert.equal(css.match(/(^|\n)\.composer \{/g)?.length, 1);
  const board = readFileSync(new URL("../src/board/Board.tsx", import.meta.url), "utf8");
  assert.ok(!/className="composer"/.test(board));
});
