// Share mode: `?film` plays the story on a fixed clock so it can be recorded.
export const FILM = typeof window !== "undefined" && new URLSearchParams(window.location.search).has("film");

type Cue = { at: number; name: string; scroll?: string };

export const CUES: Cue[] = [
  { at: 1400, name: "place:planner" },
  { at: 2300, name: "place:coder" },
  { at: 3200, name: "place:reviewer" },
  { at: 4100, name: "place:human" },
  { at: 8200, name: "remove:reviewer" },
  { at: 11200, name: "place:reviewer" },
  { at: 13600, name: "scroll", scroll: "#hundred" },
  { at: 20400, name: "scroll", scroll: "#lease" },
  { at: 21400, name: "lease:expire" },
  { at: 24600, name: "scroll", scroll: "#review" },
  { at: 25400, name: "lock:author" },
  { at: 26000, name: "lock:merge" },
  { at: 27200, name: "lock:reviewer" },
  { at: 27900, name: "lock:merge" },
  { at: 29400, name: "scroll", scroll: "#evidence" },
  { at: 30100, name: "env:seal" },
  { at: 31100, name: "env:all" },
  { at: 31700, name: "env:seal" },
  { at: 34400, name: "scroll", scroll: "#run" },
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
        if (el instanceof HTMLElement) window.scrollTo({ top: el.offsetTop, behavior: "smooth" });
      } else {
        window.dispatchEvent(new CustomEvent("maidan-film", { detail: c.name }));
      }
    }, c.at),
  );
  return () => timers.forEach((t) => window.clearTimeout(t));
}
