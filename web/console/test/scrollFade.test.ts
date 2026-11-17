import { test } from "node:test";
import assert from "node:assert/strict";
import { fadeEdges } from "../src/hooks/useScrollFade.ts";

test("a strip that fits fades neither edge", () => {
  assert.equal(fadeEdges(0, 300, 300), null);
});

test("a strip at its start fades only its end, and at its end only its start", () => {
  assert.equal(fadeEdges(0, 300, 500), "end");
  assert.equal(fadeEdges(200, 300, 500), "start");
});

test("a strip scrolled part way fades both edges, and a rounding pixel is not more content", () => {
  assert.equal(fadeEdges(100, 300, 500), "both");
  assert.equal(fadeEdges(1, 300, 301), null);
});
