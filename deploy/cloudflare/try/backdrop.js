// The gathering behind the try page's hero: when it starts, what it is told,
// and the control that stops it. The scene itself is gathering.js (built
// from try-src/gathering.ts); everything around it is here.
//
// The poster (the inline SVG of the mark in index.html) shows first, and it
// stays wherever the scene does not run: no WebGL2, a shader that fails, a
// renderer on the CPU, a context the GPU took back. The scene loads once the
// page has loaded and gone idle, so the words and the start button paint
// first; it stops itself off screen and in a hidden tab, and under reduced
// motion it is one still frame with nothing to stop. It moves, so a reader
// can stop it (WCAG 2.2.2): #gather-toggle, shown only while there is
// motion to stop.

// What the light clears behind: every block of words over the picture.
export const SHADE = ".top .brand, .top .top-link, .hero > *, .ready > *";
// The words beside the mark on a wide screen, which the field calms toward.
export const SIDE = ".hero > *";

// The states in which the picture moves or could: the control shows.
const MOVING = new Set(["running", "paused", "away"]);

export function createBackdrop(doc, win, load = () => import("./gathering.js")) {
  const host = doc.getElementById("gather");
  const canvas = host?.querySelector("canvas");
  const anchor = host?.querySelector(".gather__mark");
  if (!host || !canvas || !anchor) return null;
  const toggle = doc.getElementById("gather-toggle");
  const root = doc.documentElement;
  const motion = win.matchMedia("(prefers-reduced-motion: reduce)");
  let handle = null;
  let paused = false;

  const options = () => ({ light: root.getAttribute("data-theme") === "light", still: motion.matches, paused });

  function show(state) {
    host.dataset.state = state;
    if (toggle) toggle.hidden = !MOVING.has(state);
  }

  // The control's name says what a press will do, and changes with it
  function label() {
    if (!toggle) return;
    const text = `${paused ? "Play" : "Pause"} the animation`;
    toggle.setAttribute("aria-label", text);
    toggle.title = text;
    toggle.dataset.paused = String(paused);
  }

  async function start() {
    let scene;
    try {
      scene = await load();
    } catch {
      show("fallback");
      return;
    }
    try {
      handle = scene.mount({
        host,
        canvas,
        anchor,
        shade: () => Array.from(doc.querySelectorAll(SHADE)),
        side: () => Array.from(doc.querySelectorAll(SIDE)),
        initial: options(),
        onState: show,
      });
    } catch {
      handle = null;
    }
    if (!handle) show("fallback");
  }

  label();
  toggle?.addEventListener("click", () => {
    paused = !paused;
    label();
    handle?.update({ paused });
  });
  motion.addEventListener?.("change", () => handle?.update(options()));
  // theme.js sets the theme before the first paint; the console's switcher
  // may change it later in another tab's storage, never on this page, but a
  // change of the attribute is cheap to follow
  new win.MutationObserver(() => handle?.update(options())).observe(root, { attributes: true, attributeFilter: ["data-theme"] });

  const later = () => {
    // Safari has no idle callback; a short timeout after load does the same job
    if (typeof win.requestIdleCallback === "function") win.requestIdleCallback(() => void start(), { timeout: 1200 });
    else win.setTimeout(() => void start(), 200);
  };
  if (doc.readyState === "complete") later();
  else win.addEventListener("load", later, { once: true });

  return {
    start,
    get paused() {
      return paused;
    },
  };
}

if (typeof document !== "undefined") {
  createBackdrop(document, window);
}
