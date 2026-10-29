import type { Sim } from "./canvas";
import { draw as drawSwarm, makeWorld, resize, step as stepSwarm, type World } from "./swarm";

export const swarmSim = (agents: number, tasks: number): Sim<World> => ({
  init: (w, h) => makeWorld(w, h, agents, tasks),
  resize,
  step: (s, dt) => stepSwarm(s, dt),
  draw: drawSwarm,
});

/* Footprints: three agents each hold a block of files. With the rule on, a block cannot
   move onto files someone else holds; with it off, overlaps are merge conflicts. */
export type Foot = { w: number; h: number; cols: number; rows: number; on: boolean; boxes: { c: number; r: number; tc: number; tr: number; t: number; hue: string }[]; conflicts: number; hot: number[] };
const FW = 3;
const FH = 2;
export const footSim: Sim<Foot> = {
  init: (w, h) => ({ w, h, cols: 10, rows: 6, on: true, conflicts: 0, hot: [],
    boxes: [["103,232,249", 0, 0], ["167,139,250", 6, 3], ["252,211,77", 6, 0]].map(([hue, c, r]) => ({ c: c as number, r: r as number, tc: c as number, tr: r as number, t: Math.random(), hue: hue as string })) }),
  resize: (s, w, h) => { s.w = w; s.h = h; },
  step: (s, dt) => {
    const cells = (b: { c: number; r: number }) => { const out: number[] = []; for (let i = 0; i < FW; i++) for (let j = 0; j < FH; j++) out.push((b.r + j) * s.cols + b.c + i); return out; };
    s.boxes.forEach((b, k) => {
      b.t -= dt;
      if (b.t > 0) return;
      b.t = 0.55 + Math.random() * 0.5;
      const nc = Math.max(0, Math.min(s.cols - FW, b.c + Math.round(Math.random() * 4 - 2)));
      const nr = Math.max(0, Math.min(s.rows - FH, b.r + Math.round(Math.random() * 2 - 1)));
      if (s.on) {
        const mine = new Set(cells({ c: nc, r: nr }));
        const clash = s.boxes.some((o, m) => m !== k && cells(o).some((x) => mine.has(x)));
        if (clash) return;
      }
      b.c = nc;
      b.r = nr;
    });
    const count = new Map<number, number>();
    for (const b of s.boxes) for (const x of cells(b)) count.set(x, (count.get(x) ?? 0) + 1);
    const hot = [...count].filter(([, n]) => n > 1).map(([x]) => x);
    const fresh = hot.filter((x) => !s.hot.includes(x)).length;
    s.conflicts += fresh;
    s.hot = hot;
  },
  draw: (ctx, s, dpr) => {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, s.w, s.h);
    const pad = 14;
    const cw = (s.w - pad * 2) / s.cols;
    const ch = (s.h - pad * 2) / s.rows;
    for (let r = 0; r < s.rows; r++) for (let c = 0; c < s.cols; c++) {
      const hot = s.hot.includes(r * s.cols + c);
      ctx.fillStyle = hot ? "rgba(251,113,133,0.55)" : "rgba(255,255,255,0.045)";
      ctx.beginPath();
      ctx.roundRect(pad + c * cw + 2, pad + r * ch + 2, cw - 4, ch - 4, 5);
      ctx.fill();
    }
    for (const b of s.boxes) {
      ctx.strokeStyle = `rgba(${b.hue},0.95)`;
      ctx.fillStyle = `rgba(${b.hue},0.14)`;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.roundRect(pad + b.c * cw, pad + b.r * ch, cw * FW, ch * FH, 8);
      ctx.fill();
      ctx.stroke();
    }
  },
};

/* Review gate: changes ride to production; one in six is broken. The gate sends broken ones back. */
export type Gate = { w: number; h: number; on: boolean; items: { x: number; bad: boolean; back: number }[]; spawn: number; caught: number; incidents: number; flash: number };
export const gateSim: Sim<Gate> = {
  init: (w, h) => ({ w, h, on: true, items: [], spawn: 0, caught: 0, incidents: 0, flash: 0 }),
  resize: (s, w, h) => { s.w = w; s.h = h; },
  step: (s, dt) => {
    s.spawn -= dt;
    s.flash = Math.max(0, s.flash - dt);
    if (s.spawn <= 0) { s.items.push({ x: 0, bad: Math.random() < 0.18, back: 0 }); s.spawn = 0.45; }
    const gx = 0.62;
    for (const it of s.items) {
      if (it.back) { it.back += dt; it.x -= dt * 0.5; continue; }
      const before = it.x;
      it.x += dt * 0.22;
      if (before < gx && it.x >= gx && it.bad && s.on) { it.back = 0.001; s.caught++; }
      if (before < 0.97 && it.x >= 0.97 && it.bad) { s.incidents++; s.flash = 0.5; }
    }
    s.items = s.items.filter((it) => it.x < 1.02 && it.x > -0.05);
  },
  draw: (ctx, s, dpr) => {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, s.w, s.h);
    const y = s.h / 2;
    const gx = s.w * 0.62;
    ctx.strokeStyle = "rgba(255,255,255,0.12)";
    ctx.lineWidth = 10;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(16, y);
    ctx.lineTo(s.w - 16, y);
    ctx.stroke();
    ctx.lineWidth = 3;
    ctx.strokeStyle = s.on ? "rgba(252,211,77,0.95)" : "rgba(252,211,77,0.15)";
    ctx.setLineDash(s.on ? [] : [4, 6]);
    ctx.beginPath();
    ctx.moveTo(gx, y - 34);
    ctx.lineTo(gx, y + 34);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = s.flash ? `rgba(251,113,133,${0.25 + s.flash})` : "rgba(110,231,183,0.18)";
    ctx.beginPath();
    ctx.roundRect(s.w - 40, y - 22, 30, 44, 8);
    ctx.fill();
    for (const it of s.items) {
      const x = 16 + it.x * (s.w - 32);
      const yy = it.back ? y + 18 : y;
      ctx.fillStyle = it.bad ? "rgba(251,113,133,0.95)" : "rgba(110,231,183,0.95)";
      ctx.beginPath();
      ctx.arc(x, yy, 6, 0, Math.PI * 2);
      ctx.fill();
    }
  },
};
