import { useEffect, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { draw, makeWorld, resize, step, wastedPct, type World } from "./field";

const STATIONS = ["Queue", "Lease", "Review", "Ship"] as const;

export function Hero({ film, onPlaced }: { film: boolean; onPlaced: (on: boolean) => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const world = useRef<World | null>(null);
  const pct = useRef<HTMLSpanElement>(null);
  const hits = useRef<HTMLSpanElement>(null);
  const slotEl = useRef<HTMLSpanElement>(null);
  const chipEl = useRef<HTMLButtonElement>(null);
  const [on, setOn] = useState(false);
  const [drag, setDrag] = useState<{ x0: number; y0: number; dx: number; dy: number; moved: boolean } | null>(null);
  const [near, setNear] = useState(false);
  const [auto, setAuto] = useState<{ dx: number; dy: number } | null>(null);
  const [touched, setTouched] = useState(false);
  const base = useRef(0);

  function setMaidan(next: boolean) {
    setOn(next);
    setTouched(true);
    onPlaced(next);
    if (world.current) {
      world.current.order = next;
      base.current = next ? world.current.collisions : 0;
    }
  }

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const r0 = el.getBoundingClientRect();
    const small = r0.width < 600;
    const w = makeWorld(r0.width, r0.height, small ? 26 : 44, small ? 9 : 14);
    world.current = w;
    let raf = 0;
    let visible = true;
    let last = performance.now();
    let shown = -1;
    const fit = () => {
      const r = el.getBoundingClientRect();
      el.width = Math.round(r.width * dpr);
      el.height = Math.round(r.height * dpr);
      resize(w, r.width, r.height);
      draw(ctx, w, dpr);
    };
    fit();
    // Warm the chaos up so the first frame already shows collisions.
    for (let i = 0; i < 90; i++) step(w, 1 / 30);
    const paint = () => {
      draw(ctx, w, dpr);
      const p = wastedPct(w);
      if (p !== shown && pct.current) { shown = p; pct.current.textContent = `${p}%`; }
      if (hits.current) hits.current.textContent = String(w.collisions - base.current);
    };
    const loop = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      step(w, dt);
      paint();
      raf = visible ? requestAnimationFrame(loop) : 0;
    };
    if (reduce) {
      for (let i = 0; i < 60; i++) step(w, 1 / 30);
      paint();
    } else raf = requestAnimationFrame(loop);
    const io = new IntersectionObserver(([e]) => {
      visible = e.isIntersecting;
      if (visible && !raf && !reduce) { last = performance.now(); raf = requestAnimationFrame(loop); }
    });
    io.observe(el);
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    const settle = () => { if (reduce) { for (let i = 0; i < 120; i++) step(w, 1 / 30); paint(); } };
    el.addEventListener("maidan:settle", settle);
    return () => { cancelAnimationFrame(raf); io.disconnect(); ro.disconnect(); el.removeEventListener("maidan:settle", settle); };
  }, []);

  useEffect(() => {
    canvas.current?.dispatchEvent(new Event("maidan:settle"));
  }, [on]);

  // Share mode: a hand-free demo of the drop.
  useEffect(() => {
    if (!film) return;
    const t1 = window.setTimeout(() => {
      const a = chipEl.current?.getBoundingClientRect();
      const b = slotEl.current?.getBoundingClientRect();
      if (a && b) setAuto({ dx: b.left + b.width / 2 - (a.left + a.width / 2), dy: b.top + b.height / 2 - (a.top + a.height / 2) });
    }, 2600);
    const t2 = window.setTimeout(() => { setAuto(null); setMaidan(true); }, 4000);
    return () => { window.clearTimeout(t1); window.clearTimeout(t2); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [film]);

  function overSlot(x: number, y: number) {
    const r = slotEl.current?.getBoundingClientRect();
    if (!r) return false;
    return x > r.left - 40 && x < r.right + 40 && y > r.top - 40 && y < r.bottom + 40;
  }
  function down(e: RPointerEvent<HTMLButtonElement>) {
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ x0: e.clientX, y0: e.clientY, dx: 0, dy: 0, moved: false });
  }
  function move(e: RPointerEvent<HTMLButtonElement>) {
    if (!drag) return;
    const dx = e.clientX - drag.x0;
    const dy = e.clientY - drag.y0;
    setDrag({ ...drag, dx, dy, moved: drag.moved || Math.hypot(dx, dy) > 6 });
    setNear(overSlot(e.clientX, e.clientY));
  }
  function up(e: RPointerEvent<HTMLButtonElement>) {
    if (!drag) return;
    if (!drag.moved || overSlot(e.clientX, e.clientY)) setMaidan(true);
    setDrag(null);
    setNear(false);
  }

  const chipStyle = drag?.moved
    ? { transform: `translate(${drag.dx}px, ${drag.dy}px) scale(1.08)` }
    : auto
      ? { transform: `translate(${auto.dx}px, ${auto.dy}px)`, transition: "transform 1.3s cubic-bezier(.6,.05,.2,1)" }
      : undefined;

  return (
    <section className="hero" id="top" aria-labelledby="title">
      <p className="kicker">An explorable explanation · 4 rules · 5 minutes</p>
      <h1 id="title">
        A hundred agents.<br />
        <span className="h1-soft">Zero collisions.</span>
      </h1>
      <figure className={`instrument${on ? " is-on" : ""}`}>
        <div className="inst-top">
          <span className="live"><i aria-hidden="true" />Live · {on ? "Maidan on" : "no coordination"}</span>
          <span className="meter" aria-hidden="true">
            <span className="meter-n" ref={pct}>60%</span>
            <span className="meter-l">work wasted</span>
          </span>
        </div>
        <canvas ref={canvas} className="inst-canvas" role="img" aria-label={on ? "Agents each hold one task; tasks line up in a square" : "Agents swarm the same tasks and collide"} />
        <p className="inst-hits" aria-hidden="true">{on ? "collisions since on" : "collisions"} <span ref={hits}>0</span></p>
        <div className="dock">
          <ol className="circuit" aria-label="The swarm loop">
            {STATIONS.map((s) => (
              <li key={s} className={`node${s === "Review" ? " node-slot" : ""}`}>
                {s === "Review" ? (
                  <span ref={slotEl} className={`socket${on ? " filled" : ""}${near ? " near" : ""}`}>
                    {on ? <span className="rchip in"><span className="rdot" />Reviewer</span> : <span className="socket-hint">drop here</span>}
                  </span>
                ) : (
                  <span className="node-pill">{s}</span>
                )}
              </li>
            ))}
          </ol>
          <div className="dock-row">
            {on ? (
              <span className="rchip-ghost" aria-hidden="true" />
            ) : (
              <button
                ref={chipEl}
                type="button"
                className={`rchip grab${drag?.moved ? " dragging" : ""}${touched ? "" : " pulse"}`}
                style={chipStyle}
                onPointerDown={down}
                onPointerMove={move}
                onPointerUp={up}
                onPointerCancel={() => { setDrag(null); setNear(false); }}
                aria-label="Reviewer. Drag into the Review socket, or press to drop it in and turn Maidan on."
              >
                <span className="grip" aria-hidden="true" />
                <span className="rdot" aria-hidden="true" />
                Reviewer
                <span className="dragme" aria-hidden="true">drag me</span>
              </button>
            )}
            <button type="button" role="switch" aria-checked={on} className={`switch${on ? " on" : ""}`} onClick={() => setMaidan(!on)}>
              <span className="switch-track" aria-hidden="true"><span className="switch-knob" /></span>
              {on ? "Maidan on" : "Turn on Maidan"}
            </button>
          </div>
        </div>
        <p className="sr-only" role="status" aria-live="polite">
          {on ? "Maidan is on. Each task has one holder, review closes the loop, and wasted work falls toward zero." : "No coordination. Agents duplicate each other's work."}
        </p>
      </figure>
      <p className="dek">
        {on ? <>That's the whole trick, in one drop. <b>Here are the four rules behind it.</b></> : <>Twenty-six agents on one repo, grabbing whatever looks closest. Drop a reviewer into the loop.</>}
      </p>
      <a className="scroll-cue" href="#lease">Learn the four rules <span aria-hidden="true">↓</span></a>
    </section>
  );
}
