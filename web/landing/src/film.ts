// Share mode: `?film` plays the story on a fixed clock so it can be recorded.
export const FILM = typeof window !== "undefined" && new URLSearchParams(window.location.search).has("film");

type Cue = { at: number; name: string; scroll?: string };

export const CUES: Cue[] = [
  { at: 1000, name: "place:planner" },
  { at: 1600, name: "place:coder" },
  { at: 2200, name: "place:reviewer" },
  { at: 2900, name: "place:human" },
  { at: 5600, name: "remove:reviewer" },
  { at: 7700, name: "place:reviewer" },
  { at: 9600, name: "scroll", scroll: ".wall-frame" },
  { at: 14400, name: "scroll", scroll: "#lease .object" },
  { at: 15100, name: "lease:expire" },
  { at: 17300, name: "scroll", scroll: "#review .object" },
  { at: 17800, name: "lock:author" },
  { at: 18300, name: "lock:merge" },
  { at: 19200, name: "lock:reviewer" },
  { at: 19700, name: "lock:merge" },
  { at: 20800, name: "scroll", scroll: "#evidence .object" },
  { at: 21400, name: "env:seal" },
  { at: 22100, name: "env:all" },
  { at: 22600, name: "env:seal" },
  { at: 25000, name: "scroll", scroll: "#run" },
  { at: 28500, name: "scroll", scroll: "#run .cta.star" },
];

export function onCue(fn: (name: string) => void): () => void {
  const h = (e: Event) => fn((e as CustomEvent<string>).detail);
  window.addEventListener("maidan-film", h);
  return () => window.removeEventListener("maidan-film", h);
}

export function playFilm(): () => void {
  const timers = CUES.map((c) =>
    window.setTimeout(() => {
      if (c.scroll) {
        const el = document.querySelector(c.scroll);
        if (el instanceof HTMLElement) el.scrollIntoView({ behavior: "smooth", block: c.scroll === "#run" ? "start" : c.scroll.endsWith(".star") ? "end" : "center" });
      } else {
        window.dispatchEvent(new CustomEvent("maidan-film", { detail: c.name }));
      }
    }, c.at),
  );
  return () => timers.forEach((t) => window.clearTimeout(t));
}
