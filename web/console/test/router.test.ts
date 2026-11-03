import { test } from "node:test";
import assert from "node:assert/strict";
import { parseRoute } from "../src/shell/router.ts";

test("a hash names a place; anything else is the board", () => {
  assert.equal(parseRoute("#/needs"), "needs");
  assert.equal(parseRoute("#people"), "people");
  assert.equal(parseRoute("#/threads/t_1"), "threads");
  assert.equal(parseRoute(""), "board");
  assert.equal(parseRoute("#/nope"), "board");
});

test("the phone tab bar shows five places and puts the rest behind More", async () => {
  const { phoneTabs, ROUTES } = await import("../src/shell/router.ts");
  const { tabs, more } = phoneTabs();
  assert.equal(tabs.length, 5);
  assert.ok(more.length >= 1);
  assert.equal(tabs.length + more.length, ROUTES.length);
});
