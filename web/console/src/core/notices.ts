// Feedback: the one place a screen says what just happened. A notice shows
// as a toast for a few seconds and stays in the recent list on Tools, so a
// missed one can be read again. Notices are words, never secrets: anything
// shaped like a token is hidden before it is kept.
export type Tone = "ok" | "warn" | "error" | "info";
export type Notice = { id: number; text: string; tone: Tone; at: number };

export const KEEP = 50;
const SECRETISH = /\b[A-Za-z0-9_\-+/=.]{32,}\b/g;
const MDN = /\bmdn_[A-Za-z0-9_.-]+\b/g;
const BEARER = /\b(Bearer\s+)([^\s,;]+)/gi;

/** Hides tokens. A long token-shaped run, any `mdn_` secret, and the secret after Bearer. Ids and shas pass through. */
export function redact(text: string): string {
  return text
    .replace(MDN, "[hidden]")
    .replace(BEARER, "$1[hidden]")
    .replace(SECRETISH, (m) => (/^[0-9a-f-]{36}$/i.test(m) || /^[0-9a-f]{64}$/i.test(m) ? m : "[hidden]"));
}

/** True when the signed-in room changed, including a sign-out. The first sign-in has nothing to clear. */
export function leaveRoom(previous: string | null, next: string | null): boolean {
  return previous !== null && previous !== next;
}

export class Notices {
  private list: Notice[] = [];
  private next = 1;
  private readonly listeners = new Set<() => void>();
  subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  snapshot = (): Notice[] => this.list;
  push(text: string, tone: Tone = "info", now = Date.now()): Notice {
    const n: Notice = { id: this.next++, text: redact(text), tone, at: now };
    this.list = [n, ...this.list].slice(0, KEEP);
    for (const fn of this.listeners) fn();
    return n;
  }
  clear(): void {
    this.list = [];
    for (const fn of this.listeners) fn();
  }
}

export const notices = new Notices();

export function say(text: string, tone: Tone = "ok"): void {
  notices.push(text, tone);
}

export function sayError(e: unknown, fallback: string): void {
  notices.push(e instanceof Error && e.message ? e.message : fallback, "error");
}
