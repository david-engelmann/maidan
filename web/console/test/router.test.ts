import { test } from "node:test";
import assert from "node:assert/strict";
import { parseRoute, placeOf } from "../src/shell/router.ts";

test("a hash names a place; no hash is the board", () => {
  assert.equal(parseRoute("#/needs"), "needs");
  assert.equal(parseRoute("#people"), "people");
  assert.equal(parseRoute("#/threads/t_1"), "threads");
  assert.equal(parseRoute(""), "board");
  assert.equal(parseRoute("#"), "board");
  assert.equal(parseRoute("#/"), "board");
  assert.equal(parseRoute("#/board?channel=c1"), "board");
});

test("a hash that names no place is not found, not the board", () => {
  assert.equal(parseRoute("#/nope"), "not-found");
  // Files is the artifacts place's title, not its route.
  assert.equal(parseRoute("#/files"), "not-found");
  assert.equal(parseRoute("#/files/abc?x=1"), "not-found");
  assert.equal(placeOf("#/files/abc?x=1"), "files");
});

test("the phone tab bar shows five places and puts the rest behind More", async () => {
  const { phoneTabs, ROUTES } = await import("../src/shell/router.ts");
  const { tabs, more } = phoneTabs();
  assert.equal(tabs.length, 5);
  assert.ok(more.length >= 1);
  assert.equal(tabs.length + more.length, ROUTES.length);
});
