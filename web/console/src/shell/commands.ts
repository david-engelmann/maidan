// Commands for the palette and the keyboard: plain data, ranked by a small
// fuzzy match, plus two-key "g then letter" jumps between places.
import { ROUTES, type RouteId } from "./router.ts";

export type Command = { id: string; title: string; hint?: string; group: "Go to" | "Actions"; keys?: string[]; run: () => void };

/** The letter after "g" that jumps to each place. */
export const GO_KEYS: Record<string, RouteId> = { b: "board", n: "needs", t: "threads", p: "people", o: "tools", f: "artifacts" };
const KEY_OF = Object.fromEntries(Object.entries(GO_KEYS).map(([k, r]) => [r, k])) as Record<RouteId, string>;

export function routeCommands(go: (id: RouteId) => void): Command[] {
  return ROUTES.map((r) => ({ id: `go-${r.id}`, title: r.title, hint: r.hint, group: "Go to", keys: ["G", KEY_OF[r.id].toUpperCase()], run: () => go(r.id) }));
}

/** How well `query` matches `text`: higher is better, null is no match.
 *  Prefix beats word start beats a scattered subsequence. */
export function score(text: string, query: string): number | null {
  const t = text.toLowerCase();
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  if (t.startsWith(q)) return 1000 - t.length;
  const word = t.indexOf(` ${q}`);
  if (word >= 0) return 800 - word;
  const at = t.indexOf(q);
  if (at >= 0) return 600 - at;
  let i = 0;
  let gaps = 0;
  let last = -1;
  for (const ch of q) {
    const next = t.indexOf(ch, i);
    if (next < 0) return null;
    if (last >= 0) gaps += next - last - 1;
    last = next;
    i = next + 1;
  }
  return 300 - gaps;
}

/** Commands that match, best first; the title counts more than the hint. */
export function rank(commands: Command[], query: string): Command[] {
  if (!query.trim()) return commands;
  const scored: { c: Command; s: number; i: number }[] = [];
  commands.forEach((c, i) => {
    const title = score(c.title, query);
    const hint = c.hint ? score(c.hint, query) : null;
    // Two tiers: any title match outranks every hint-only match.
    const s = title != null ? 10000 + title : hint != null ? hint : -Infinity;
    if (s > -Infinity) scored.push({ c, s, i });
  });
  return scored.sort((a, b) => b.s - a.s || a.i - b.i).map((x) => x.c);
}

/** Feed keys in; returns the place to go once "g" then a letter arrive
 *  within `windowMs`, and the state to carry to the next key. */
export function goSequence(state: { at: number } | null, key: string, now: number, windowMs = 1200): { state: { at: number } | null; go: RouteId | null } {
  const k = key.toLowerCase();
  if (state && now - state.at <= windowMs && GO_KEYS[k]) return { state: null, go: GO_KEYS[k] };
  if (k === "g") return { state: { at: now }, go: null };
  return { state: null, go: null };
}

/** True when a key press is typing into a field, so shortcuts stay out of the way. */
export function isTyping(target: EventTarget | null): boolean {
  const el = target as { tagName?: string; isContentEditable?: boolean } | null;
  if (!el?.tagName) return false;
  return el.isContentEditable === true || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName);
}

// "New message" from anywhere: Threads may not be mounted yet, so the ask is
// held until it mounts (takeCompose) or delivered live if it already is.
let composePending = false;
export function requestCompose(): void {
  composePending = true;
  window.dispatchEvent(new Event("maidan:compose"));
}
export function takeCompose(): boolean {
  const was = composePending;
  composePending = false;
  return was;
}

/** j and k step through a list; returns the next index, clamped to the ends.
 *  With nothing focused yet, j picks the first row and k the last. */
export function listStep(current: number, length: number, key: string): number | null {
  if (!length) return null;
  const k = key.toLowerCase();
  if (k !== "j" && k !== "k") return null;
  if (current < 0) return k === "j" ? 0 : length - 1;
  return Math.min(length - 1, Math.max(0, current + (k === "j" ? 1 : -1)));
}

/** The rows j and k move between, in screen order. */
export const LIST_ROWS = ".convs .conv, .person-row, .need-pick, .need-card, .lane .task:not(.task-sk)";
