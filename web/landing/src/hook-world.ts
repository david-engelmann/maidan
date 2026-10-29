// The opening swarm: agents chase tasks with no coordination until Maidan is switched on.
import { roundRect } from "./loop";

export interface HAgent { x: number; y: number; vx: number; vy: number; task: number; hue: number; tr: number[] }
export interface HTask { x: number; y: number; tx: number; ty: number; p: number; workers: number; was: number; holder: number; done: number; clash: number }
export interface HWorld {
  w: number;
  h: number;
  portrait: boolean;
  agents: HAgent[];
  tasks: HTask[];
  order: boolean;
  shipped: number;
  collisions: number;
  work: number;
  waste: number;
  t: number;
}

const rnd = (a: number, b: number) => a + Math.random() * (b - a);

function field(w: number, h: number, portrait: boolean) {
  if (portrait) return { x0: w * 0.07, x1: w * 0.93, y0: h * (h < 720 ? 0.6 : 0.54), y1: h * 0.9 };
  return { x0: w * 0.5, x1: w * 0.95, y0: h * 0.16, y1: h * 0.86 };
}

function newTask(wd: HWorld): HTask {
  const f = field(wd.w, wd.h, wd.portrait);
  const x = rnd(f.x0 + 12, f.x1 - 12);
  const y = rnd(f.y0 + 12, f.y1 - 12);
  return { x, y, tx: x, ty: y, p: 0, workers: 0, was: 0, holder: -1, done: 0, clash: 0 };
}

export function makeWorld(w: number, h: number): HWorld {
  const portrait = w < 760;
  const wd: HWorld = { w, h, portrait, agents: [], tasks: [], order: false, shipped: 0, collisions: 0, work: 0, waste: 0, t: 0 };
  const f = field(w, h, portrait);
  const na = portrait ? 22 : 34;
  const nt = portrait ? 10 : 15;
  for (let i = 0; i < na; i++) wd.agents.push({ x: rnd(f.x0, f.x1), y: rnd(f.y0, f.y1), vx: rnd(-40, 40), vy: rnd(-40, 40), task: -1, hue: i % 3, tr: [] });
  for (let i = 0; i < nt; i++) wd.tasks.push(newTask(wd));
  return wd;
}

export function resize(wd: HWorld, w: number, h: number) {
  if (w === wd.w && h === wd.h) return;
  const a = field(wd.w, wd.h, wd.portrait);
  wd.w = w;
  wd.h = h;
  wd.portrait = w < 760;
  const b = field(w, h, wd.portrait);
  const map = (x: number, y: number): [number, number] => [
    b.x0 + ((x - a.x0) / (a.x1 - a.x0)) * (b.x1 - b.x0),
    b.y0 + ((y - a.y0) / (a.y1 - a.y0)) * (b.y1 - b.y0),
  ];
  for (const ag of wd.agents) {
    [ag.x, ag.y] = map(ag.x, ag.y);
    ag.tr = [];
  }
  for (const t of wd.tasks) {
    [t.x, t.y] = map(t.x, t.y);
    [t.tx, t.ty] = map(t.tx, t.ty);
  }
}

function slot(wd: HWorld, i: number, n: number) {
  const f = field(wd.w, wd.h, wd.portrait);
  const cols = wd.portrait ? 3 : 5;
  const rows = Math.ceil(n / cols);
  const gx = Math.min((f.x1 - f.x0) / cols, wd.portrait ? 96 : 104);
  const gy = Math.min((f.y1 - f.y0) / rows, wd.portrait ? 66 : 104);
  const cx = (f.x0 + f.x1) / 2;
  const cy = (f.y0 + f.y1) / 2;
  const c = i % cols;
  const r = Math.floor(i / cols);
  return { x: cx + (c - (cols - 1) / 2) * gx, y: cy + (r - (rows - 1) / 2) * gy };
}

export function step(wd: HWorld, dt: number) {
  wd.t += dt;
  const { agents, tasks } = wd;
  const f = field(wd.w, wd.h, wd.portrait);
  for (const t of tasks) t.workers = 0;
  if (wd.order) {
    tasks.forEach((t, i) => {
      const s = slot(wd, i, tasks.length);
      t.tx = s.x;
      t.ty = s.y;
    });
    for (let i = 0; i < agents.length; i++) {
      const a = agents[i];
      if (a.task >= 0 && tasks[a.task]?.holder !== i) a.task = -1;
      if (a.task < 0) {
        const k = tasks.findIndex((t) => t.holder < 0 && t.done === 0);
        if (k >= 0) {
          tasks[k].holder = i;
          a.task = k;
        }
      }
    }
  } else {
    for (const t of tasks) t.holder = -1;
    for (const a of agents) {
      if (a.task < 0 || Math.random() < dt * 0.35) {
        let best = -1;
        let bd = Infinity;
        for (let k = 0; k < tasks.length; k++) {
          if (tasks[k].done) continue;
          const d = Math.hypot(tasks[k].x - a.x, tasks[k].y - a.y) * rnd(0.5, 1.5);
          if (d < bd) {
            bd = d;
            best = k;
          }
        }
        a.task = best;
      }
    }
  }
  for (const t of tasks) {
    t.x += (t.tx - t.x) * Math.min(1, dt * 4);
    t.y += (t.ty - t.y) * Math.min(1, dt * 4);
  }
  for (let i = 0; i < agents.length; i++) {
    const a = agents[i];
    const t = a.task >= 0 ? tasks[a.task] : null;
    let ax = 0;
    let ay = 0;
    if (t && !t.done) {
      const dx = t.x - a.x;
      const dy = t.y - a.y;
      const d = Math.hypot(dx, dy) || 1;
      if (d < 16) {
        t.workers++;
        ax = -a.vx * 5 + (wd.order ? Math.cos(wd.t * 3 + i) * 60 : 0);
        ay = -a.vy * 5 + (wd.order ? Math.sin(wd.t * 3 + i) * 60 : 0);
      } else {
        ax = (dx / d) * 320;
        ay = (dy / d) * 320;
      }
    } else if (wd.order) {
      // Idle agents wait on the bench beside the board.
      const bx = wd.portrait ? f.x0 + ((i * 37) % (f.x1 - f.x0)) : f.x0 - 30;
      const by = wd.portrait ? f.y1 + 18 : f.y0 + ((i * 41) % (f.y1 - f.y0));
      ax = (bx - a.x) * 4;
      ay = (by - a.y) * 4;
    } else {
      ax = rnd(-120, 120);
      ay = rnd(-120, 120);
    }
    if (!wd.order) {
      ax += rnd(-260, 260);
      ay += rnd(-260, 260);
    }
    a.vx = (a.vx + ax * dt) * (1 - dt * 2.4);
    a.vy = (a.vy + ay * dt) * (1 - dt * 2.4);
    a.x = Math.max(4, Math.min(wd.w - 4, a.x + a.vx * dt));
    a.y = Math.max(f.y0 - 40, Math.min(wd.h - 4, a.y + a.vy * dt));
  }
  const decay = Math.exp(-dt / 1.1);
  let work = 0;
  let waste = 0;
  for (let k = 0; k < tasks.length; k++) {
    const t = tasks[k];
    if (t.done) {
      t.done += dt;
      if (t.done > 0.7) {
        tasks[k] = newTask(wd);
        for (const a of agents) if (a.task === k) a.task = -1;
      }
      continue;
    }
    if (t.workers > 1 && t.was <= 1) {
      wd.collisions++;
      t.clash = 1;
    }
    t.was = t.workers;
    t.clash = Math.max(0, t.clash - dt * 1.6);
    if (t.workers > 0) {
      t.p += dt * 0.42;
      work += t.workers;
      waste += t.workers - 1;
    }
    if (t.p >= 1) {
      t.done = 0.0001;
      t.holder = -1;
      wd.shipped++;
    }
  }
  wd.work = wd.work * decay + work * dt;
  wd.waste = wd.waste * decay + waste * dt;
}

export function wastePct(wd: HWorld): number {
  return wd.work > 0.02 ? Math.round((wd.waste / wd.work) * 100) : 0;
}

const HUES = ["45,212,191", "167,139,250", "251,191,36"];

export function draw(ctx: CanvasRenderingContext2D, wd: HWorld, dpr: number, motion: boolean) {
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, wd.w, wd.h);
  // Ambient light over the field: red while it thrashes, teal once ordered.
  const f = field(wd.w, wd.h, wd.portrait);
  const cx = (f.x0 + f.x1) / 2;
  const cy = (f.y0 + f.y1) / 2;
  const rad = Math.max(f.x1 - f.x0, f.y1 - f.y0) * 0.85;
  const glow = ctx.createRadialGradient(cx, cy, 0, cx, cy, rad);
  glow.addColorStop(0, wd.order ? "rgba(45,212,191,0.13)" : "rgba(244,63,94,0.12)");
  glow.addColorStop(0.55, wd.order ? "rgba(167,139,250,0.05)" : "rgba(251,191,36,0.04)");
  glow.addColorStop(1, "rgba(0,0,0,0)");
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, wd.w, wd.h);
  ctx.lineCap = "round";
  const s = wd.portrait ? 18 : 22;
  // Holder lines.
  if (wd.order) {
    ctx.lineWidth = 1;
    for (const a of wd.agents) {
      const t = a.task >= 0 ? wd.tasks[a.task] : null;
      if (!t || t.done) continue;
      ctx.strokeStyle = "rgba(45,212,191,0.28)";
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(t.x, t.y);
      ctx.stroke();
    }
  }
  for (const t of wd.tasks) {
    const x = t.x - s / 2;
    const y = t.y - s / 2;
    const clash = !wd.order && t.workers > 1;
    if (t.done) {
      const k = t.done / 0.7;
      ctx.strokeStyle = `rgba(74,222,128,${1 - k})`;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(t.x, t.y, s * 0.6 + k * 26, 0, Math.PI * 2);
      ctx.stroke();
      continue;
    }
    if (clash || t.clash > 0) {
      const g = Math.max(t.clash, clash ? 0.6 : 0);
      ctx.fillStyle = `rgba(244,63,94,${0.16 * g})`;
      ctx.beginPath();
      ctx.arc(t.x, t.y, s * 1.5, 0, Math.PI * 2);
      ctx.fill();
    }
    roundRect(ctx, x, y, s, s, 6);
    ctx.fillStyle = "rgba(255,255,255,0.05)";
    ctx.fill();
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = clash ? "rgba(244,63,94,0.95)" : t.holder >= 0 ? "rgba(45,212,191,0.9)" : "rgba(255,255,255,0.22)";
    ctx.stroke();
    // Progress fills from the bottom.
    if (t.p > 0) {
      ctx.save();
      roundRect(ctx, x, y, s, s, 6);
      ctx.clip();
      ctx.fillStyle = clash ? "rgba(244,63,94,0.35)" : "rgba(45,212,191,0.32)";
      ctx.fillRect(x, y + s * (1 - t.p), s, s * t.p);
      ctx.restore();
    }
    if (wd.order && t.holder >= 0) {
      ctx.strokeStyle = "rgba(251,191,36,0.9)";
      ctx.lineWidth = 1.6;
      ctx.beginPath();
      ctx.arc(t.x, t.y, s * 0.82, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * (1 - ((wd.t * 0.33) % 1) * 0.25));
      ctx.stroke();
    }
  }
  for (const a of wd.agents) {
    const c = HUES[a.hue];
    const t = a.task >= 0 ? wd.tasks[a.task] : null;
    const bad = !wd.order && !!t && t.workers > 1 && Math.hypot(t.x - a.x, t.y - a.y) < 20;
    const col = bad ? "244,63,94" : c;
    if (motion) {
      a.tr.push(a.x, a.y);
      if (a.tr.length > 18) a.tr.splice(0, 2);
      for (let k = 2; k < a.tr.length; k += 2) {
        ctx.strokeStyle = `rgba(${col},${(k / a.tr.length) * 0.4})`;
        ctx.lineWidth = 1 + (k / a.tr.length) * 2.2;
        ctx.beginPath();
        ctx.moveTo(a.tr[k - 2], a.tr[k - 1]);
        ctx.lineTo(a.tr[k], a.tr[k + 1]);
        ctx.stroke();
      }
    }
    ctx.fillStyle = `rgb(${col})`;
    ctx.beginPath();
    ctx.arc(a.x, a.y, wd.portrait ? 3.2 : 3.6, 0, Math.PI * 2);
    ctx.fill();
  }
}
