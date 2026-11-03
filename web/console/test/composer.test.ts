import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

test("the board's new-task form and the thread composer do not share a class", () => {
  // They once did, and the board's flex-wrap rule shrank the thread's message field to a pill.
  assert.equal(css.match(/(^|\n)\.composer \{/g)?.length, 1);
  const board = readFileSync(new URL("../src/board/Board.tsx", import.meta.url), "utf8");
  assert.ok(!/className="composer"/.test(board));
});
