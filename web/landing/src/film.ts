import { useEffect, useRef } from "react";

/** `?film` turns the page into a hands-free demo for recording share clips. */
export const FILM = typeof location !== "undefined" && new URLSearchParams(location.search).has("film");

const wait = (ms: number) => new Promise((r) => window.setTimeout(r, ms));

function focus(id: string) {
  const el = document.getElementById(id)?.querySelector(".fig-body") ?? document.getElementById(id);
  if (!el) return;
  const r = el.getBoundingClientRect();
  const top = window.scrollY + r.top - Math.max(16, (window.innerHeight - r.height) / 2);
  window.scrollTo({ top, behavior: "smooth" });
}

/** Figures register a scripted move under their own name. */
export function useFilm(name: string, act: () => void) {
  const fn = useRef(act);
  useEffect(() => {
    fn.current = act;
  });
  useEffect(() => {
    if (!FILM) return;
    const h = () => fn.current();
    window.addEventListener(`film:${name}`, h);
    return () => window.removeEventListener(`film:${name}`, h);
  }, [name]);
}

export async function runFilm() {
  document.documentElement.classList.add("film");
  await wait(6000);
  for (const [id, hold] of [["lease", 4400], ["footprints", 4000], ["review", 4600], ["evidence", 4600]] as const) {
    focus(id);
    await wait(900);
    window.dispatchEvent(new Event(`film:${id}`));
    await wait(hold);
  }
  focus("finale");
  await wait(700);
  window.dispatchEvent(new Event("film:finale"));
  await wait(3600);
  document.querySelector(".know")?.scrollIntoView({ behavior: "smooth", block: "start" });
  await wait(2600);
  document.documentElement.dataset.filmDone = "1";
}
