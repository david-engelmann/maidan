// By day the water is a pool set into the paper (journey.css). On a wide
// screen it stands beside the hero's words, as tall as they are, and clear of
// the headline, which may run past the words' column. Only a measurement
// knows where the headline ends, so the box is measured here and handed to
// CSS as --pool-*; the still shows in it, and journey.ts draws the live pool
// into the same box, so the one never jumps when the other takes over.

const WIDE = 1080;

export function watchHeroPool(): () => void {
  const hero = document.querySelector<HTMLElement>("main > .hero");
  const copy = hero?.querySelector<HTMLElement>(".hero-copy");
  const headline = copy?.querySelector("h1");
  if (!hero || !copy || !headline) return () => {};
  const place = () => {
    const w = document.documentElement.clientWidth;
    const heroBox = hero.getBoundingClientRect();
    const words = copy.getBoundingClientRect();
    // the headline's ink, not the block that holds it
    const range = document.createRange();
    range.selectNodeContents(headline);
    const ink = range.getBoundingClientRect();
    const top = words.top + 6;
    const left = Math.max(Math.max(words.right, ink.right) + 56, w * 0.5);
    const right = w - words.left;
    const bottom = Math.max(words.bottom - 4, top + 380);
    if (w < WIDE || right - left < 200) {
      hero.removeAttribute("data-pool");
      return;
    }
    hero.style.setProperty("--pool-l", `${left - heroBox.left}px`);
    hero.style.setProperty("--pool-t", `${top - heroBox.top}px`);
    hero.style.setProperty("--pool-w", `${right - left}px`);
    hero.style.setProperty("--pool-h", `${bottom - top}px`);
    hero.setAttribute("data-pool", "");
  };
  place();
  const watch = new ResizeObserver(place);
  watch.observe(hero);
  watch.observe(copy);
  let live = true;
  // the headline's width changes when the page's own face arrives
  void document.fonts?.ready.then(() => {
    if (live) place();
  });
  return () => {
    live = false;
    watch.disconnect();
  };
}
