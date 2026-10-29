// ?film: plays the whole story hands-free for recording launch videos.
import { emit } from "./loop";

const sleep = (ms: number) => new Promise((r) => window.setTimeout(r, ms));

function glide(to: number, ms: number) {
  return new Promise<void>((done) => {
    const from = window.scrollY;
    const t0 = performance.now();
    const f = (now: number) => {
      const t = Math.min(1, (now - t0) / ms);
      const e = t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
      window.scrollTo(0, from + (to - from) * e);
      if (t < 1) requestAnimationFrame(f);
      else done();
    };
    requestAnimationFrame(f);
  });
}

const top = (sel: string, offset = 0) => {
  const el = document.querySelector(sel);
  return el ? el.getBoundingClientRect().top + window.scrollY + offset : 0;
};

export async function playFilm() {
  document.documentElement.classList.add("is-film");
  window.scrollTo(0, 0);
  await sleep(2600);
  emit("hook:on");
  await sleep(2300);
  await glide(top("#build", -12), 1100);
  for (const r of ["planner", "coder", "coder", "reviewer", "human"]) {
    await sleep(r === "planner" ? 300 : 560);
    emit("circuit:add", r);
  }
  await sleep(1500);
  emit("circuit:rule", "leases");
  await sleep(2300);
  emit("circuit:rule", "leases");
  emit("circuit:rule", "reviewer");
  await sleep(2300);
  emit("circuit:rule", "reviewer");
  const scenes = ["home", "lease", "gate", "evidence", "swarm"];
  for (const s of scenes) {
    const el = document.querySelector<HTMLElement>(`[data-scene="${s}"]`);
    if (!el) continue;
    const y = el.getBoundingClientRect().top + window.scrollY + el.offsetHeight / 2 - window.innerHeight / 2;
    await glide(y, 900);
    await sleep(s === "swarm" ? 3400 : 1700);
  }
  document.documentElement.classList.add("film-done");
  document.documentElement.dataset.filmDone = "1";
}
