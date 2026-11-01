import assert from "node:assert/strict";
import { test } from "node:test";
import { LANE_PAGE, windowed } from "../src/core/board.ts";

test("short lanes show everything", () => {
  assert.deepEqual(windowed([1, 2, 3], LANE_PAGE), { shown: [1, 2, 3], hidden: 0 });
});

test("long lanes show the first page and count the rest", () => {
  const cards = Array.from({ length: LANE_PAGE * 2 + 5 }, (_, i) => i);
  const w = windowed(cards, LANE_PAGE);
  assert.equal(w.shown.length, LANE_PAGE);
  assert.equal(w.shown[0], 0);
  assert.equal(w.hidden, LANE_PAGE + 5);
  assert.equal(windowed(cards, LANE_PAGE * 3).hidden, 0);
});

test("a nonsense limit shows nothing rather than throwing", () => {
  assert.deepEqual(windowed([1, 2], -4), { shown: [], hidden: 2 });
});
