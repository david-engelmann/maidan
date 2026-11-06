/**
 * Pure console UI helpers for launch-quality safety and accessibility.
 * No React / DOM globals required at import time — callers pass values in,
 * and tests run under `node --experimental-strip-types --test` with stubs.
 */

const UNSAFE_SCHEMES = /^(javascript|data|vbscript|file|about):/i;

/** True when a URL is safe to open in a new tab as an http(s) link. */
export function isSafeExternalHref(href: string): boolean {
  const raw = href.trim();
  if (!raw) return false;
  if (UNSAFE_SCHEMES.test(raw)) return false;
  // Protocol-relative and bare schemes we do not allow for message/body links.
  if (raw.startsWith("//")) return false;
  try {
    const u = new URL(raw);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * Returns a normalized http(s) href, or `null` when the value must not be
 * rendered as a navigable link (javascript:, data:, relative junk, etc.).
 */
export function safeExternalHref(href: string): string | null {
  const raw = href.trim();
  if (!isSafeExternalHref(raw)) return null;
  try {
    return new URL(raw).toString();
  } catch {
    return null;
  }
}

/** `rel` for every console link that opens away from the app. */
export function externalLinkRel(): string {
  return "noopener noreferrer";
}

export type CopyResult = { ok: true } | { ok: false; reason: string };

type ClipboardLike = { writeText(text: string): Promise<void> };

/**
 * Copies text through a clipboard-like surface. Empty strings refuse so a
 * blank secret can never look like a successful copy.
 */
export async function copyText(
  text: string,
  clipboard?: ClipboardLike | null,
): Promise<CopyResult> {
  if (typeof text !== "string" || text.length === 0) {
    return { ok: false, reason: "empty" };
  }
  const clip =
    clipboard ??
    (typeof navigator !== "undefined"
      ? (navigator.clipboard as ClipboardLike | undefined)
      : undefined);
  if (!clip || typeof clip.writeText !== "function") {
    return { ok: false, reason: "unavailable" };
  }
  try {
    await clip.writeText(text);
    return { ok: true };
  } catch {
    return { ok: false, reason: "denied" };
  }
}

/** Compact relative label for recent activity (English, stable for tests). */
export function formatRelativeTime(thenMs: number, nowMs: number): string {
  if (!Number.isFinite(thenMs) || !Number.isFinite(nowMs)) return "unknown time";
  const delta = Math.round((nowMs - thenMs) / 1000);
  const abs = Math.abs(delta);
  const future = delta < 0;
  const ago = (unit: string, n: number) =>
    future ? `in ${n} ${unit}${n === 1 ? "" : "s"}` : `${n} ${unit}${n === 1 ? "" : "s"} ago`;
  if (abs < 5) return future ? "in a moment" : "just now";
  if (abs < 60) return ago("second", abs);
  const mins = Math.round(abs / 60);
  if (mins < 60) return ago("minute", mins);
  const hours = Math.round(mins / 60);
  if (hours < 48) return ago("hour", hours);
  const days = Math.round(hours / 24);
  if (days < 14) return ago("day", days);
  const weeks = Math.round(days / 7);
  if (weeks < 9) return ago("week", weeks);
  return formatAbsoluteTime(thenMs);
}

/** Stable absolute timestamp for `title=` / screen-reader fallbacks (UTC). */
export function formatAbsoluteTime(ms: number): string {
  if (!Number.isFinite(ms)) return "unknown time";
  return new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z");
}

export type LivePoliteness = "off" | "polite" | "assertive";

/** Map a UI urgency to an `aria-live` politeness. */
export function livePoliteness(kind: "silent" | "status" | "alert"): LivePoliteness {
  if (kind === "silent") return "off";
  if (kind === "alert") return "assertive";
  return "polite";
}

export type Focusable = {
  focus: (opts?: { preventScroll?: boolean }) => void;
  matches?: (sel: string) => boolean;
  closest?: (sel: string) => Focusable | null;
  getAttribute?: (name: string) => string | null;
  tagName?: string;
  isContentEditable?: boolean;
};

/** Capture the active element so a modal/sheet can restore it on close. */
export function captureFocus(getActive: () => Focusable | null | undefined): Focusable | null {
  return getActive() ?? null;
}

/** Restore focus without scrolling the page out from under the reader. */
export function restoreFocus(el: Focusable | null | undefined): boolean {
  if (!el || typeof el.focus !== "function") return false;
  try {
    el.focus({ preventScroll: true });
    return true;
  } catch {
    try {
      el.focus();
      return true;
    } catch {
      return false;
    }
  }
}

const TYPING_SELECTOR =
  "input, textarea, select, [contenteditable=''], [contenteditable='true'], [role='textbox']";

/** True when keyboard shortcuts must stand down for typing. */
export function isTypingTarget(el: Focusable | null | undefined): boolean {
  if (!el) return false;
  if (el.isContentEditable) return true;
  const tag = (el.tagName || "").toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return true;
  if (typeof el.matches === "function") {
    try {
      if (el.matches(TYPING_SELECTOR)) return true;
    } catch {
      /* happy-dom / stubs may not implement full selectors */
    }
  }
  if (typeof el.closest === "function") {
    try {
      if (el.closest("[contenteditable='true'], [role='textbox']")) return true;
    } catch {
      /* ignore */
    }
  }
  const role = el.getAttribute?.("role");
  return role === "textbox" || role === "searchbox" || role === "combobox";
}

/** Pair reduced-motion with the matching ScrollBehavior (mirrors motion.ts). */
export function scrollBehaviorFor(reduceMotion: boolean): "auto" | "smooth" {
  return reduceMotion ? "auto" : "smooth";
}

/** Empty / whitespace titles fall back so empty states never announce blank. */
export function visibleLabel(text: string, fallback: string): string {
  const t = text.trim();
  const f = fallback.trim();
  if (t) return t;
  return f || "Untitled";
}
