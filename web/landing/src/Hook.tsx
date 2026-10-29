import { useEffect, useRef, useState } from "react";
import { draw, makeWorld, resize, step, wastePct, type HWorld } from "./hook-world";
import { FILM, fitCanvas, on, runWhileVisible } from "./loop";

export function Hook({ reduce }: { reduce: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const world = useRef<HWorld | null>(null);
  const wasteEl = useRef<HTMLElement>(null);
  const shipEl = useRef<HTMLElement>(null);
  const colEl = useRef<HTMLElement>(null);
  const [on_, setOn] = useState(false);
  const [say, setSay] = useState("");

  useEffect(() => {
    const c = canvas.current;
    const ctx = c?.getContext("2d");
    if (!c || !ctx) return;
    let acc = 0;
    let lastSay = "";
    return runWhileVisible(c, (dt) => {
      const { w, h, dpr } = fitCanvas(c);
      if (!world.current) world.current = makeWorld(w, h);
      const wd = world.current;
      resize(wd, w, h);
      step(wd, reduce ? dt * 0.5 : dt);
      draw(ctx, wd, dpr, !reduce);
      acc += dt;
      if (acc > 0.2) {
        acc = 0;
        const pct = wastePct(wd);
        if (wasteEl.current) wasteEl.current.textContent = `${pct}%`;
        if (shipEl.current) shipEl.current.textContent = String(wd.shipped);
        if (colEl.current) colEl.current.textContent = String(wd.collisions);
        wasteEl.current?.parentElement?.classList.toggle("is-good", pct === 0);
        // The screen reader line changes only every few seconds.
        const next = `${pct}% of effort wasted. ${wd.shipped} shipped.`;
        if (Math.floor(wd.t) % 4 === 0 && next !== lastSay) {
          lastSay = next;
          setSay(next);
        }
      }
    });
  }, [reduce]);

  const flip = (v: boolean) => {
    setOn(v);
    if (world.current) world.current.order = v;
  };
  useEffect(() => on("hook:on", () => flip(true)), []);

  return (
    <section className="hook" aria-labelledby="hook-title">
      <canvas ref={canvas} className="hook-canvas" aria-hidden="true" />
      <div className="hook-copy">
        <p className="eyebrow">Open source · self-hosted</p>
        <h1 id="hook-title">
          Coordination for <span className="grad">agent swarms.</span>
        </h1>
        <p className="lede">One holder per task. Review before merge. Proof on close.</p>
        <div className="meters">
          <div className={`meter ${on_ ? "" : "is-bad"}`}>
            <b ref={wasteEl}>0%</b>
            <span>effort wasted</span>
          </div>
          <div className="meter">
            <b ref={colEl}>0</b>
            <span>collisions</span>
          </div>
          <div className="meter ok">
            <b ref={shipEl}>0</b>
            <span>shipped</span>
          </div>
        </div>
        <button type="button" role="switch" aria-checked={on_} className={`power ${on_ ? "is-on" : ""}`} onClick={() => flip(!on_)}>
          <span className="power-track" aria-hidden="true">
            <span className="power-knob" />
          </span>
          <span className="power-label">{on_ ? "Maidan is on" : "Turn on Maidan"}</span>
        </button>
        <p className="hook-note" aria-live="polite">
          {on_ ? "Every task has one holder. Nothing is built twice." : "No coordination: agents pile onto the same task."}
        </p>
        <span className="sr-only" aria-live="polite">
          {FILM ? "" : say}
        </span>
      </div>
      <a className="scroll-cue" href="#build">
        Build one yourself ↓
      </a>
    </section>
  );
}
