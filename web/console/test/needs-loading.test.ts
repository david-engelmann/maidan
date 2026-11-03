import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// Needs you loads like Threads: placeholder rows the size of real ones, so the
// list does not jump from one line of text to a column of cards.
test("Needs you shows skeleton rows while it loads, not a line of text", () => {
  const src = readFileSync(new URL("../src/needs/NeedsYou.tsx", import.meta.url), "utf8");
  const at = src.indexOf('data.status === "loading" &&');
  assert.ok(at >= 0, "the loading branch exists");
  const branch = src.slice(at, src.indexOf(")}", at));
  assert.match(branch, /role="status"/);
  assert.match(branch, /aria-label="Loading what needs you"/);
  assert.equal(branch.match(/className="skeleton row-sk"/g)?.length, 3);
  assert.doesNotMatch(branch, /<p className="muted">Loading/);
});
