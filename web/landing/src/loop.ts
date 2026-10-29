// Shared frame loop: runs only while the element is on screen and the tab is visible.
export function runWhileVisible(el: Element, frame: (dt: number, now: number) => void): () => void {
  let raf = 0;
  let last = 0;
  let inView = false;
  const tick = (now: number) => {
    const dt = Math.min(0.05, Math.max(0, (now - last) / 1000));
    last = now;
    frame(dt, now);
    raf = requestAnimationFrame(tick);
  };
  const sync = () => {
    const go = inView && !document.hidden;
    if (go && !raf) {
      last = performance.now();
      raf = requestAnimationFrame(tick);
    } else if (!go && raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
  };
  const io = new IntersectionObserver(([e]) => {
    inView = e.isIntersecting;
    sync();
  });
  io.observe(el);
  document.addEventListener("visibilitychange", sync);
  return () => {
    io.disconnect();
    document.removeEventListener("visibilitychange", sync);
    cancelAnimationFrame(raf);
    raf = 0;
  };
}

export interface Fit {
  w: number;
  h: number;
  dpr: number;
}

// Match the canvas backing store to its CSS size, capped at 2x.
export function fitCanvas(c: HTMLCanvasElement): Fit {
  const w = c.clientWidth;
  const h = c.clientHeight;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const W = Math.max(1, Math.round(w * dpr));
  const H = Math.max(1, Math.round(h * dpr));
  if (c.width !== W) c.width = W;
  if (c.height !== H) c.height = H;
  return { w, h, dpr };
}

export const FILM = new URLSearchParams(location.search).has("film");

// A tiny event bus so the film director can drive each section.
type Handler = (arg?: string) => void;
const handlers = new Map<string, Set<Handler>>();
export function on(name: string, fn: Handler): () => void {
  const set = handlers.get(name) ?? new Set<Handler>();
  set.add(fn);
  handlers.set(name, set);
  return () => set.delete(fn);
}
export function emit(name: string, arg?: string) {
  handlers.get(name)?.forEach((fn) => fn(arg));
}

export function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}
