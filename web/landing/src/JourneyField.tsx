import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Handle, State } from "./journey";
import { MediaToggle } from "./MediaToggle";
import { usePageTheme, usePrefersReducedMotion } from "./stageshow/shared";
import "./journey.css";

/** The states in which the scene is drawing and the poster steps aside. */
const DRAWING: readonly string[] = ["running", "paused", "still", "away"];

/**
 * The journey behind the page: liquid light, hot beside the hero's words,
 * at rest behind the sections, hot again under the closing call to action.
 * Decoration only: aria-hidden, under the words, and without a GPU the
 * hero's poster of it simply stays.
 *
 * The scene is its own chunk (journey.js), loaded once the page has loaded
 * and gone idle, so the words paint first. The canvas is fixed behind the
 * page and sized by CSS, so nothing moves when it arrives. It moves, so a
 * reader can stop it (WCAG 2.2.2); under reduced motion it is a composed
 * still per act and there is nothing to stop.
 */
export function JourneyField() {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const handle = useRef<Handle | null>(null);
  const [state, setState] = useState<State | "poster">("poster");
  const [paused, setPaused] = useState(false);
  const light = usePageTheme() === "light";
  const still = usePrefersReducedMotion();
  const opts = useRef({ light, still, paused });

  useEffect(() => {
    opts.current = { light, still, paused };
    handle.current?.update(opts.current);
  }, [light, still, paused]);

  // The page's layout and its poster follow the scene from <html>
  const drawing = DRAWING.includes(state);
  useEffect(() => {
    const root = document.documentElement;
    if (drawing) root.setAttribute("data-journey", "on");
    else root.removeAttribute("data-journey");
    return () => root.removeAttribute("data-journey");
  }, [drawing]);

  useEffect(() => {
    const el = host.current;
    const cv = canvas.current;
    if (!el || !cv) return;
    let gone = false;
    const start = () => {
      import("./journey")
        .then((m) => {
          if (gone) return;
          handle.current = m.mount(el, cv, opts.current, (s) => setState(s));
          if (!handle.current) setState("fallback");
        })
        .catch(() => setState("fallback"));
    };
    let idle = 0;
    const later = () => {
      // Safari has no idle callback; a short timeout after load does the same job
      idle = typeof window.requestIdleCallback === "function" ? window.requestIdleCallback(start, { timeout: 1200 }) : window.setTimeout(start, 200);
    };
    if (document.readyState === "complete") later();
    else window.addEventListener("load", later, { once: true });
    return () => {
      gone = true;
      window.removeEventListener("load", later);
      if (typeof window.cancelIdleCallback === "function") window.cancelIdleCallback(idle);
      window.clearTimeout(idle);
      handle.current?.destroy();
      handle.current = null;
    };
  }, []);

  // The control lives in the page's header bar (App's #journey-control):
  // fixed over the page, it covered the end of a line of copy as it scrolled
  // by on a phone. The bar is there long before the scene draws; without it
  // the control stands fixed in the corner (journey.css).
  const toggle = drawing && !still ? (
    <MediaToggle paused={paused} onToggle={() => setPaused((p) => !p)} noun="animation" className="journey-toggle" />
  ) : null;
  const bar = toggle ? document.getElementById("journey-control") : null;
  return (
    <>
      <div ref={host} className="journey" data-state={state} aria-hidden="true">
        <canvas ref={canvas} className="journey__canvas" />
      </div>
      {bar && toggle ? createPortal(toggle, bar) : toggle}
    </>
  );
}
