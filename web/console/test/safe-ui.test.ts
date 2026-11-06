import { test } from "node:test";
import assert from "node:assert/strict";
import {
  captureFocus,
  copyText,
  externalLinkRel,
  formatAbsoluteTime,
  formatRelativeTime,
  isSafeExternalHref,
  isTypingTarget,
  livePoliteness,
  restoreFocus,
  safeExternalHref,
  scrollBehaviorFor,
  visibleLabel,
} from "../src/core/safeUi.ts";

test("safeExternalHref allows only http(s) and rejects dangerous schemes", () => {
  assert.equal(safeExternalHref("https://maidan.world/ui/"), "https://maidan.world/ui/");
  assert.equal(isSafeExternalHref("http://127.0.0.1:8080/health"), true);
  assert.equal(safeExternalHref("javascript:alert(1)"), null);
  assert.equal(safeExternalHref("data:text/html,hi"), null);
  assert.equal(safeExternalHref("vbscript:msg"), null);
  assert.equal(safeExternalHref("//evil.example/path"), null);
  assert.equal(safeExternalHref("/relative"), null);
  assert.equal(safeExternalHref(""), null);
  assert.equal(safeExternalHref("  "), null);
  assert.equal(safeExternalHref("file:///etc/passwd"), null);
});

test("externalLinkRel always carries noopener noreferrer", () => {
  assert.equal(externalLinkRel(), "noopener noreferrer");
});

test("copyText refuses empty and reports clipboard outcomes", async () => {
  assert.deepEqual(await copyText(""), { ok: false, reason: "empty" });
  assert.deepEqual(await copyText("secret", null), { ok: false, reason: "unavailable" });
  const ok = { writeText: async () => {} };
  assert.deepEqual(await copyText("secret", ok), { ok: true });
  const denied = {
    writeText: async () => {
      throw new Error("denied");
    },
  };
  assert.deepEqual(await copyText("secret", denied), { ok: false, reason: "denied" });
});

test("formatRelativeTime covers near, mid, and far ranges", () => {
  const now = Date.parse("2026-11-06T19:00:00Z");
  assert.equal(formatRelativeTime(now, now), "just now");
  assert.equal(formatRelativeTime(now - 30_000, now), "30 seconds ago");
  assert.equal(formatRelativeTime(now - 5 * 60_000, now), "5 minutes ago");
  assert.equal(formatRelativeTime(now - 3 * 3600_000, now), "3 hours ago");
  assert.equal(formatRelativeTime(now - 3 * 86400_000, now), "3 days ago");
  assert.equal(formatRelativeTime(now + 45_000, now), "in 45 seconds");
  assert.equal(formatRelativeTime(now - 14 * 86400_000, now), "2 weeks ago");
  assert.equal(formatRelativeTime(now - 70 * 86400_000, now), formatAbsoluteTime(now - 70 * 86400_000));
  assert.equal(formatRelativeTime(Number.NaN, now), "unknown time");
});

test("formatAbsoluteTime is stable UTC without millis", () => {
  assert.equal(formatAbsoluteTime(Date.parse("2026-11-06T19:00:00.123Z")), "2026-11-06T19:00:00Z");
  assert.equal(formatAbsoluteTime(Number.NaN), "unknown time");
});

test("livePoliteness maps silent/status/alert", () => {
  assert.equal(livePoliteness("silent"), "off");
  assert.equal(livePoliteness("status"), "polite");
  assert.equal(livePoliteness("alert"), "assertive");
});

test("captureFocus and restoreFocus round-trip without scroll", () => {
  const calls: unknown[] = [];
  const el = {
    focus(opts?: { preventScroll?: boolean }) {
      calls.push(opts);
    },
  };
  assert.equal(captureFocus(() => el), el);
  assert.equal(captureFocus(() => null), null);
  assert.equal(restoreFocus(el), true);
  assert.deepEqual(calls, [{ preventScroll: true }]);
  assert.equal(restoreFocus(null), false);
});

test("isTypingTarget stands down shortcuts in fields", () => {
  assert.equal(isTypingTarget(null), false);
  assert.equal(isTypingTarget({ focus() {}, tagName: "DIV" }), false);
  assert.equal(isTypingTarget({ focus() {}, tagName: "INPUT" }), true);
  assert.equal(isTypingTarget({ focus() {}, tagName: "TEXTAREA" }), true);
  assert.equal(isTypingTarget({ focus() {}, tagName: "SELECT" }), true);
  assert.equal(isTypingTarget({ focus() {}, tagName: "DIV", isContentEditable: true }), true);
  assert.equal(
    isTypingTarget({
      focus() {},
      tagName: "DIV",
      getAttribute: (n) => (n === "role" ? "textbox" : null),
    }),
    true,
  );
  assert.equal(
    isTypingTarget({
      focus() {},
      tagName: "DIV",
      matches: (sel) => sel.includes("[role='textbox']"),
    }),
    true,
  );
});

test("scrollBehaviorFor honors reduced motion", () => {
  assert.equal(scrollBehaviorFor(true), "auto");
  assert.equal(scrollBehaviorFor(false), "smooth");
});

test("visibleLabel never announces blank", () => {
  assert.equal(visibleLabel("  Board  ", "Untitled"), "Board");
  assert.equal(visibleLabel("   ", "Nothing here yet"), "Nothing here yet");
  assert.equal(visibleLabel("", "  "), "Untitled");
});
