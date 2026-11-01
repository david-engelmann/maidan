import assert from "node:assert/strict";
import { test } from "node:test";
import { FOCUSABLE, trapTarget } from "../src/shell/focus.ts";

test("Tab wraps from last to first, Shift+Tab from first to last", () => {
  assert.equal(trapTarget(3, 2, false), 0);
  assert.equal(trapTarget(3, 0, true), 2);
});

test("in the middle the browser moves focus", () => {
  assert.equal(trapTarget(3, 1, false), null);
  assert.equal(trapTarget(3, 1, true), null);
});

test("focus outside the dialog is pulled back in; an empty dialog is left alone", () => {
  assert.equal(trapTarget(3, -1, false), 0);
  assert.equal(trapTarget(3, -1, true), 2);
  assert.equal(trapTarget(0, -1, false), null);
});

test("fields count as focusable, so a sheet with inputs still traps", () => {
  for (const t of ["input", "select", "textarea"]) assert.ok(FOCUSABLE.includes(t));
});
