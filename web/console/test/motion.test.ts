import assert from "node:assert/strict";
import { test } from "node:test";
import { reducedMotion, scrollBehavior } from "../src/core/motion.ts";

test("a scroll jumps when the reader asked for less motion", () => {
  assert.equal(scrollBehavior(true), "auto");
  assert.equal(scrollBehavior(false), "smooth");
});

test("reduced motion is off when the browser has no media query", () => {
  assert.equal(typeof matchMedia, "undefined");
  assert.equal(reducedMotion(), false);
});
