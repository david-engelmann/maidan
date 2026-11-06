/** How a scroll should move. Reduced motion jumps; otherwise it eases. */
export function scrollBehavior(reduce: boolean): ScrollBehavior {
  return reduce ? "auto" : "smooth";
}

/** The reader's reduced-motion setting at the moment of a scroll. */
export function reducedMotion(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}
