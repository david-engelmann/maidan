import { useEffect, useRef } from "react";

type Agent = { x: number; y: number; hx: number; hy: number; vx: number; vy: number; hold: number; hue: number; name: string };

const HUES = [168, 22, 262, 44, 200];
const NAMES = ["Ada", "Bo", "Cy", "Dee", "Eli", "Fen", "Gus", "Hal", "Ivy", "Jo", "Kai", "Lu"];

function rand(seed: number) {
  const x = Math.sin(seed * 999.13) * 43758.5453;
  return x - Math.floor(x);
}

export function Swarm({ count, tall, dark = false }: { count: number; tall: boolean; dark?: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const label = useRef<HTMLParagraphElement>(null);
  const counter = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const ctx = el.getContext("2d");
    if (!ctx) return;
    const ink = dark ? "236,240,248" : "22,24,29";
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let w = 0;
    let h = 0;
    let raf = 0;
    let visible = false;
    let done = 0;
    let lastDone = -1;
    const pointer = { x: -999, y: -999, down: false };
    const agents: Agent[] = [];
    const fit = () => {
      const r = el.getBoundingClientRect();
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      w = r.width;
      h = r.height;
      el.width = Math.round(w * dpr);
      el.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      agents.length = 0;
      const R = Math.min(w, h) * 0.42;
      for (let i = 0; i < count; i++) {
        const ring = count > 30 ? 0.45 + 0.55 * Math.sqrt(rand(i + 1)) : 0.78 + 0.22 * rand(i + 3);
        const a = (i / count) * Math.PI * 2 + rand(i + 7) * 0.3;
        const rx = Math.cos(a) * R * ring * (w > h ? 1.5 : 1);
        const ry = Math.sin(a) * R * ring;
        agents.push({
          x: w / 2 + rx,
          y: h / 2 + ry,
          hx: w / 2 + rx,
          hy: h / 2 + ry,
          vx: 0,
          vy: 0,
          hold: rand(i + 11) < 0.5 ? rand(i + 5) * 4 : -rand(i + 9) * 3,
          hue: HUES[i % HUES.length],
          name: NAMES[i % NAMES.length] + (i >= NAMES.length ? String(Math.floor(i / NAMES.length)) : ""),
        });
      }
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    const io = new IntersectionObserver(([e]) => {
      visible = e.isIntersecting;
      if (visible && !raf) raf = requestAnimationFrame(loop);
    });
    io.observe(el);

    let last = performance.now();
    function draw(dt: number) {
      if (!ctx) return;
      ctx.clearRect(0, 0, w, h);
      const cx = w / 2;
      const cy = h / 2;
      // queue core: crisp ring with a slow tick
      const spin = (performance.now() / 4000) % 1;
      ctx.strokeStyle = `rgba(${ink},0.16)`;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(cx, cy, 26, 0, Math.PI * 2);
      ctx.stroke();
      ctx.strokeStyle = `rgba(${ink},0.7)`;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(cx, cy, 26, spin * Math.PI * 2, spin * Math.PI * 2 + 0.9);
      ctx.stroke();
      ctx.fillStyle = dark ? "#eef1f6" : "#16181d";
      ctx.beginPath();
      ctx.arc(cx, cy, 15, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = dark ? "#0b0d12" : "#f4f1ea";
      ctx.font = "600 8px ui-monospace, Menlo, monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText("QUEUE", cx, cy + 0.5);
      let nearest: Agent | null = null;
      let nd = 28 * 28;
      let holding = 0;
      for (const a of agents) {
        // physics: spring home, repel from pointer
        const dx = a.x - pointer.x;
        const dy = a.y - pointer.y;
        const d2 = dx * dx + dy * dy;
        if (d2 < 90 * 90 && d2 > 1) {
          const f = (pointer.down ? 2400 : 900) / d2;
          a.vx += dx * f * dt;
          a.vy += dy * f * dt;
        }
        if (d2 < nd) {
          nd = d2;
          nearest = a;
        }
        a.vx += (a.hx - a.x) * 6 * dt;
        a.vy += (a.hy - a.y) * 6 * dt;
        a.vx *= 0.9;
        a.vy *= 0.9;
        a.x += a.vx * dt * 8;
        a.y += a.vy * dt * 8;
        // lifecycle: negative = idle countdown, positive = holding lease
        if (!reduce) {
          if (a.hold < 0) {
            a.hold += dt;
            if (a.hold >= 0) a.hold = 2.5 + ((a.hx * 7 + a.hy) % 4);
          } else {
            a.hold -= dt;
            if (a.hold <= 0) {
              a.hold = -0.6 - ((a.hy * 3) % 2);
              done += 1;
            }
          }
        }
        const busy = a.hold > 0;
        if (busy) holding += 1;
        if (busy) {
          ctx.strokeStyle = `hsla(${a.hue}, 70%, ${dark ? 62 : 45}%, ${dark ? 0.42 : 0.22})`;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(cx, cy);
          const mx = (cx + a.x) / 2 + (a.y - cy) * 0.12;
          const my = (cy + a.y) / 2 - (a.x - cx) * 0.12;
          ctx.quadraticCurveTo(mx, my, a.x, a.y);
          ctx.stroke();
          // pulse travelling out
          const t = (performance.now() / 1400 + a.hx * 0.01) % 1;
          const px = (1 - t) * (1 - t) * cx + 2 * (1 - t) * t * mx + t * t * a.x;
          const py = (1 - t) * (1 - t) * cy + 2 * (1 - t) * t * my + t * t * a.y;
          ctx.fillStyle = `hsla(${a.hue}, 80%, ${dark ? 66 : 50}%, ${dark ? 0.9 : 0.7})`;
          ctx.beginPath();
          ctx.arc(px, py, dark ? 2.2 : 1.6, 0, Math.PI * 2);
          ctx.fill();
        }
        const r = dark ? 4.8 : count > 30 ? 3.4 : 5.5;
        ctx.fillStyle = busy ? `hsl(${a.hue}, 78%, ${dark ? 62 : 44}%)` : `rgba(${ink},0.28)`;
        ctx.beginPath();
        ctx.arc(a.x, a.y, r, 0, Math.PI * 2);
        if (dark && busy) {
          ctx.shadowColor = `hsl(${a.hue}, 90%, 60%)`;
          ctx.shadowBlur = 10;
        }
        ctx.fill();
        ctx.shadowBlur = 0;
        if (busy) {
          ctx.strokeStyle = `hsla(${a.hue}, 72%, 44%, 0.5)`;
          ctx.lineWidth = 1.2;
          ctx.beginPath();
          ctx.arc(a.x, a.y, r + 3, -Math.PI / 2, -Math.PI / 2 + Math.min(1, a.hold / 4) * Math.PI * 2);
          ctx.stroke();
        }
      }
      if (label.current) {
        if (nearest && pointer.x > -900) {
          const busy = nearest.hold > 0;
          label.current.textContent = busy
            ? `${nearest.name} holds task-${Math.floor(nearest.hx) % 900} · lease ${nearest.hold.toFixed(1)}s · footprint src/${["auth", "queue", "api", "lease", "ui"][Math.floor(nearest.hy) % 5]}/`
            : `${nearest.name} is idle, waiting for the next claim`;
          label.current.style.opacity = "1";
        } else label.current.style.opacity = "0";
      }
      if (counter.current && done !== lastDone) {
        lastDone = done;
        counter.current.textContent = `${holding} holding · ${done} closed`;
      }
    }
    function loop(now: number) {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      draw(dt);
      raf = visible && !reduce ? requestAnimationFrame(loop) : 0;
    }
    const pos = (e: PointerEvent) => {
      const r = el.getBoundingClientRect();
      pointer.x = e.clientX - r.left;
      pointer.y = e.clientY - r.top;
      if (reduce) draw(0);
    };
    const down = (e: PointerEvent) => {
      pointer.down = true;
      pos(e);
    };
    const up = () => {
      pointer.down = false;
    };
    const leave = () => {
      pointer.x = -999;
      pointer.y = -999;
      pointer.down = false;
      if (reduce) draw(0);
    };
    el.addEventListener("pointermove", pos);
    el.addEventListener("pointerdown", down);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointerleave", leave);
    draw(0);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      io.disconnect();
      el.removeEventListener("pointermove", pos);
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointerleave", leave);
    };
  }, [count, dark]);

  return (
    <div className={`swarm${tall ? " tall" : ""}${dark ? " dark" : ""}`}>
      <canvas ref={canvas} className="swarm-canvas" aria-label={`${count} agents claiming tasks from one queue`} role="img" />
      <p className="swarm-label" ref={label} aria-hidden="true" />
      <p className="swarm-count">
        <span ref={counter}>{count} agents</span>
      </p>
      <p className="swarm-zero" aria-label="Zero collisions">
        <b>0</b>
        <span>collisions</span>
      </p>
    </div>
  );
}
