// Share mode: `?film` plays the story on a fixed clock so it can be recorded.
export const FILM = typeof window !== "undefined" && new URLSearchParams(window.location.search).has("film");

type Cue = { at: number; name: string; scroll?: string };

export const CUES: Cue[] = [
  { at: 1200, name: "place:planner" },
  { at: 2000, name: "place:coder" },
  { at: 2800, name: "place:reviewer" },
  { at: 3600, name: "place:human" },
  { at: 7000, name: "remove:reviewer" },
  { at: 9600, name: "place:reviewer" },
  { at: 12000, name: "scroll", scroll: ".wall-frame" },
  { at: 18000, name: "scroll", scroll: "#lease .object" },
  { at: 18900, name: "lease:expire" },
  { at: 21600, name: "scroll", scroll: "#review .object" },
  { at: 22300, name: "lock:author" },
  { at: 22900, name: "lock:merge" },
  { at: 24000, name: "lock:reviewer" },
  { at: 24600, name: "lock:merge" },
  { at: 26000, name: "scroll", scroll: "#evidence .object" },
  { at: 26700, name: "env:seal" },
  { at: 27600, name: "env:all" },
  { at: 28200, name: "env:seal" },
  { at: 31200, name: "scroll", scroll: "#run" },
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
        if (el instanceof HTMLElement) el.scrollIntoView({ behavior: "smooth", block: c.scroll === "#run" ? "start" : "center" });
      } else {
        window.dispatchEvent(new CustomEvent("maidan-film", { detail: c.name }));
      }
    }, c.at),
  );
  return () => timers.forEach((t) => window.clearTimeout(t));
}
