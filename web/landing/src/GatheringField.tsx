import { useEffect, useRef, useState } from "react";
import type { Handle, State, Variant } from "./gathering";
import { usePageTheme, usePrefersReducedMotion } from "./stageshow/shared";
import "./gathering.css";

/**
 * The gathering behind the hero, or its echo under the closing call to
 * action. Decoration only: aria-hidden, under the words, and without WebGL
 * the poster of the mark simply stays.
 *
 * The scene is its own chunk (gathering.js). The hero's loads once the page
 * has loaded and gone idle, so the words paint first; the close's loads as
 * it nears the screen. Its box is the hero's or the section's own, sized by
 * CSS, so nothing moves when it arrives.
 */
export function GatheringField({ variant, paused, shade }: { variant: Variant; paused: boolean; shade: () => Element[] }) {
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const mark = useRef<HTMLElement>(null);
  const handle = useRef<Handle | null>(null);
  const [state, setState] = useState<State | "poster">("poster");
  const light = usePageTheme() === "light";
  const still = usePrefersReducedMotion();
  const opts = useRef({ light, still, paused });

  useEffect(() => {
    opts.current = { light, still, paused };
    handle.current?.update(opts.current);
  }, [light, still, paused]);

  useEffect(() => {
    const el = host.current;
    const cv = canvas.current;
    const mk = mark.current;
    if (!el || !cv || !mk) return;
    let gone = false;
    const start = () => {
      import("./gathering")
        .then((m) => {
          if (gone) return;
          handle.current = m.mount(el, cv, mk, shade, variant, opts.current, (s) => setState(s));
          if (!handle.current) setState("fallback");
        })
        .catch(() => setState("fallback"));
    };
    let cancel = () => {};
    if (variant === "close") {
      const io = new IntersectionObserver(
        (entries) => {
          if (!entries.some((e) => e.isIntersecting)) return;
          io.disconnect();
          start();
        },
        { rootMargin: "60% 0px" },
      );
      io.observe(el);
      cancel = () => io.disconnect();
    } else {
      let idle = 0;
      const later = () => {
        // Safari has no idle callback; a short timeout after load does the same job
        idle = typeof window.requestIdleCallback === "function" ? window.requestIdleCallback(start, { timeout: 1200 }) : window.setTimeout(start, 200);
      };
      if (document.readyState === "complete") later();
      else window.addEventListener("load", later, { once: true });
      cancel = () => {
        window.removeEventListener("load", later);
        if (typeof window.cancelIdleCallback === "function") window.cancelIdleCallback(idle);
        window.clearTimeout(idle);
      };
    }
    return () => {
      gone = true;
      cancel();
      handle.current?.destroy();
      handle.current = null;
    };
  }, [variant, shade]);

  return (
    <div ref={host} className={`gather gather--${variant}`} data-state={state} aria-hidden="true">
      <canvas ref={canvas} className="gather__canvas" />
      <i ref={mark} className="gather__mark" />
    </div>
  );
}
