// A strip that scrolls sideways with its scrollbar hidden (the Tools tabs,
// Activity's filters on a phone) says nothing about what is off screen. This
// marks which edge has more behind it, `data-fade="start" | "end" | "both"`,
// and the stylesheet fades that edge.
import { useEffect, type RefObject } from "react";

/** The edges of a sideways strip with more content past them. */
export function fadeEdges(scrollLeft: number, clientWidth: number, scrollWidth: number): "start" | "end" | "both" | null {
  const start = scrollLeft > 1;
  const end = scrollWidth - clientWidth - scrollLeft > 1;
  return start && end ? "both" : start ? "start" : end ? "end" : null;
}

export function useScrollFade(ref: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => {
      const edges = fadeEdges(el.scrollLeft, el.clientWidth, el.scrollWidth);
      if (edges) el.dataset.fade = edges;
      else delete el.dataset.fade;
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    const resized = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    resized?.observe(el);
    return () => {
      el.removeEventListener("scroll", update);
      resized?.disconnect();
    };
  }, [ref]);
}
