import assert from "node:assert/strict";
import { test } from "node:test";
import { STALE_AFTER_MS, staleBanner } from "../src/core/board.ts";
import { LiveStatusStore } from "../src/core/bus.ts";

const NOW = Date.parse("2026-11-01T12:00:00Z");

test("no banner before the first load or while live", () => {
  assert.equal(staleBanner({ kind: "reconnecting" }, null, null, NOW), null);
  assert.equal(staleBanner({ kind: "connected" }, NOW - 10 * STALE_AFTER_MS, null, NOW), null);
  assert.equal(staleBanner({ kind: "connecting" }, NOW, null, NOW), null);
});

test("a failing refresh always shows, even while connected", () => {
  assert.match(staleBanner({ kind: "connected" }, NOW - 1000, NOW, NOW) ?? "", /^Refresh failing/);
});

test("down live updates: reconnecting first, then out of date, refused says so", () => {
  assert.match(staleBanner({ kind: "reconnecting" }, NOW - 1000, null, NOW) ?? "", /Reconnecting/);
  assert.match(staleBanner({ kind: "closed" }, NOW - STALE_AFTER_MS - 1, null, NOW) ?? "", /out of date/);
  assert.match(staleBanner({ kind: "refused" }, NOW, null, NOW) ?? "", /refused/);
});

test("the live status store notifies on change only", () => {
  const s = new LiveStatusStore();
  let n = 0;
  s.subscribe(() => n++);
  const st = { kind: "connected" as const };
  s.set(st);
  s.set(st);
  assert.equal(n, 1);
  assert.equal(s.getSnapshot(), st);
});
