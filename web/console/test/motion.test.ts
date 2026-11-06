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

test("a throwing matchMedia is treated as no reduced-motion preference", () => {
  const prior = globalThis.matchMedia;
  // @ts-expect-error test double
  globalThis.matchMedia = () => {
    throw new Error("matchMedia unavailable");
  };
  try {
    assert.equal(reducedMotion(), false);
  } finally {
    if (prior) globalThis.matchMedia = prior;
    else delete (globalThis as { matchMedia?: typeof matchMedia }).matchMedia;
  }
});
