import { useEffect, useRef } from "react";

/** `?film` turns the page into a hands-free demo for recording share clips. */
export const FILM = typeof location !== "undefined" && new URLSearchParams(location.search).has("film");

const wait = (ms: number) => new Promise((r) => window.setTimeout(r, ms));

function focus(id: string) {
  const sec = document.getElementById(id);
  if (!sec) return;
  const fig = sec.querySelector(".fig-body") ?? sec;
  const vh = window.innerHeight;
  const head = sec.querySelector("h2") ?? fig;
  const top = head.getBoundingClientRect().top;
  const bottom = fig.getBoundingClientRect().bottom;
  const y = bottom - top + 48 <= vh ? top - (vh - (bottom - top)) / 2 : bottom - vh + 24;
  window.scrollTo({ top: window.scrollY + y, behavior: "smooth" });
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
  await wait(5600);
  for (const [id, hold] of [["lease", 4200], ["footprints", 3800], ["review", 4400], ["evidence", 4400]] as const) {
    focus(id);
    await wait(900);
    window.dispatchEvent(new Event(`film:${id}`));
    await wait(hold);
  }
  focus("finale");
  await wait(700);
  window.dispatchEvent(new Event("film:finale"));
  await wait(3400);
  document.querySelector(".know")?.scrollIntoView({ behavior: "smooth", block: "start" });
  await wait(2600);
  document.documentElement.dataset.filmDone = "1";
}
