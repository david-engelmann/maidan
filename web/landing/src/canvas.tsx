import { useEffect, useRef } from "react";

export type Sim<S> = {
  init: (w: number, h: number) => S;
  resize?: (s: S, w: number, h: number) => void;
  step: (s: S, dt: number) => void;
  draw: (ctx: CanvasRenderingContext2D, s: S, dpr: number) => void;
};

/** One canvas, sized to its box at device pixel ratio, stepped only while on screen. */
export function SimCanvas<S>({ sim, stateRef, label, className, reduce, onFrame }: {
  sim: Sim<S>;
  stateRef: { current: S | null };
  label: string;
  className?: string;
  reduce: boolean;
  onFrame?: (s: S) => void;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  const frameCb = useRef(onFrame);
  useEffect(() => {
    frameCb.current = onFrame;
  }, [onFrame]);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    let dpr = Math.min(window.devicePixelRatio || 1, 2);
    let raf = 0;
    let visible = false;
    let last = 0;
    let ticks = 0;
    const fit = () => {
      const r = canvas.getBoundingClientRect();
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = Math.round(r.width * dpr);
      canvas.height = Math.round(r.height * dpr);
      if (!stateRef.current) stateRef.current = sim.init(r.width, r.height);
      else sim.resize?.(stateRef.current, r.width, r.height);
    };
    fit();
    const loop = (now: number) => {
      const s = stateRef.current;
      if (!s) return;
      const dt = Math.min(0.05, last ? (now - last) / 1000 : 0.016);
      last = now;
      sim.step(s, dt);
      sim.draw(ctx, s, dpr);
      if (++ticks % 6 === 0) frameCb.current?.(s);
      if (visible && !reduce) raf = requestAnimationFrame(loop);
    };
    const still = () => {
      const s = stateRef.current;
      if (!s) return;
      for (let i = 0; i < 240; i++) sim.step(s, 1 / 60);
      for (let i = 0; i < 12; i++) sim.draw(ctx, s, dpr);
      frameCb.current?.(s);
    };
    const io = new IntersectionObserver(([e]) => {
      visible = e.isIntersecting;
      cancelAnimationFrame(raf);
      last = 0;
      if (visible && !reduce) raf = requestAnimationFrame(loop);
      if (visible && reduce) still();
    });
    io.observe(canvas);
    const ro = new ResizeObserver(fit);
    ro.observe(canvas);
    const poke = () => { if (reduce) still(); };
    canvas.addEventListener("sim:poke", poke);
    return () => {
      cancelAnimationFrame(raf);
      io.disconnect();
      ro.disconnect();
      canvas.removeEventListener("sim:poke", poke);
    };
  }, [sim, stateRef, reduce]);
  return <canvas ref={ref} className={className} role="img" aria-label={label} />;
}
