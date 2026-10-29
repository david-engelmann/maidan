export type Agent = { x: number; y: number; vx: number; vy: number; task: number; hue: number };
export type Task = { id: number; x: number; y: number; tx: number; ty: number; p: number; workers: number; holder: number; done: number };
export type World = { w: number; h: number; agents: Agent[]; tasks: Task[]; next: number; shipped: number; wasted: number; work: number; order: boolean; t: number };

const rnd = (a: number, b: number) => a + Math.random() * (b - a);

export function makeWorld(w: number, h: number, nAgents: number, nTasks: number): World {
  const world: World = { w, h, agents: [], tasks: [], next: 1, shipped: 0, wasted: 0, work: 0, order: false, t: 0 };
  for (let i = 0; i < nAgents; i++) world.agents.push({ x: rnd(0, w), y: rnd(0, h), vx: rnd(-1, 1), vy: rnd(-1, 1), task: -1, hue: i % 3 });
  for (let i = 0; i < nTasks; i++) world.tasks.push(newTask(world));
  return world;
}

function newTask(world: World): Task {
  const x = rnd(world.w * 0.08, world.w * 0.92);
  const y = rnd(world.h * 0.12, world.h * 0.88);
  return { id: world.next++, x, y, tx: x, ty: y, p: 0, workers: 0, holder: -1, done: 0 };
}

export function resize(world: World, w: number, h: number) {
  const sx = w / world.w;
  const sy = h / world.h;
  for (const a of world.agents) { a.x *= sx; a.y *= sy; }
  for (const t of world.tasks) { t.x *= sx; t.y *= sy; t.tx *= sx; t.ty *= sy; }
  world.w = w;
  world.h = h;
}

/** Grid slots of the square, the queue every task lines up in once Maidan is on. */
function slot(world: World, i: number, n: number) {
  const cols = Math.ceil(Math.sqrt(n * (world.w / world.h)));
  const rows = Math.ceil(n / cols);
  const size = world.w >= 900 ? Math.min(world.w * 0.5, world.h * 0.55) : Math.min(world.w * 0.78, world.h * 0.4);
  const cw = size / Math.max(cols - 1, 1);
  const ch = (size * (rows / cols)) / Math.max(rows - 1, 1);
  const c = i % cols;
  const r = Math.floor(i / cols);
  const cx = world.w >= 900 ? world.w * 0.66 : world.w / 2;
  const cy = world.w >= 900 ? world.h / 2 : world.h * 0.6;
  return { x: cx - size / 2 + c * cw, y: cy - (ch * (rows - 1)) / 2 + r * ch };
}

export function step(world: World, dt: number, speed = 1) {
  world.t += dt;
  const { agents, tasks } = world;
  for (const t of tasks) t.workers = 0;
  if (world.order) {
    tasks.forEach((t, i) => {
      const s = slot(world, i, tasks.length);
      t.tx = s.x;
      t.ty = s.y;
    });
    // Leases: a task has one holder; idle agents claim the first unheld task.
    for (let i = 0; i < agents.length; i++) {
      const a = agents[i];
      if (a.task >= 0 && tasks[a.task]?.holder !== i) a.task = -1;
      if (a.task < 0) {
        const k = tasks.findIndex((t) => t.holder < 0 && t.done === 0);
        if (k >= 0) { tasks[k].holder = i; a.task = k; }
      }
    }
  } else {
    for (const t of tasks) t.holder = -1;
    // No coordination: each agent chases whatever looks closest, and retargets on a whim.
    for (const a of agents) {
      if (a.task < 0 || Math.random() < dt * 0.25) {
        let best = -1;
        let bd = Infinity;
        for (let k = 0; k < tasks.length; k++) {
          if (tasks[k].done) continue;
          const d = Math.hypot(tasks[k].x - a.x, tasks[k].y - a.y) * rnd(0.6, 1.4);
          if (d < bd) { bd = d; best = k; }
        }
        a.task = best;
      }
    }
  }
  for (const t of tasks) {
    t.x += (t.tx - t.x) * Math.min(1, dt * 3);
    t.y += (t.ty - t.y) * Math.min(1, dt * 3);
  }
  for (const a of agents) {
    const t = a.task >= 0 ? tasks[a.task] : null;
    let ax = 0;
    let ay = 0;
    if (t && !t.done) {
      const dx = t.x - a.x;
      const dy = t.y - a.y;
      const d = Math.hypot(dx, dy) || 1;
      if (d < 14) {
        t.workers++;
        ax = -a.vx * 4;
        ay = -a.vy * 4;
      } else {
        ax = (dx / d) * 220;
        ay = (dy / d) * 220;
      }
    } else {
      ax = rnd(-80, 80);
      ay = rnd(-80, 80);
    }
    if (!world.order) { ax += rnd(-160, 160); ay += rnd(-160, 160); }
    a.vx = (a.vx + ax * dt) * (1 - dt * 2.2);
    a.vy = (a.vy + ay * dt) * (1 - dt * 2.2);
    a.x += a.vx * dt * speed;
    a.y += a.vy * dt * speed;
    a.x = Math.max(4, Math.min(world.w - 4, a.x));
    a.y = Math.max(4, Math.min(world.h - 4, a.y));
  }
  for (let k = 0; k < tasks.length; k++) {
    const t = tasks[k];
    if (t.done) {
      t.done += dt;
      if (t.done > 0.6) {
        tasks[k] = newTask(world);
        for (const a of agents) if (a.task === k) a.task = -1;
      }
      continue;
    }
    if (t.workers > 0) {
      t.p += dt * 0.45;
      world.work += dt * t.workers;
      world.wasted += dt * (t.workers - 1);
    }
    if (t.p >= 1) { t.done = 0.0001; t.holder = -1; world.shipped++; }
  }
}

const HUES = ["103,232,249", "167,139,250", "110,231,183"];

const TRAILS = new WeakMap<Agent, number[]>();

export function draw(ctx: CanvasRenderingContext2D, world: World, dpr: number) {
  const { w, h } = world;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.lineCap = "round";
  for (const a of world.agents) {
    let tr = TRAILS.get(a);
    if (!tr) { tr = []; TRAILS.set(a, tr); }
    tr.push(a.x, a.y);
    if (tr.length > 20) tr.splice(0, 2);
    const c = HUES[a.hue];
    for (let k = 2; k < tr.length; k += 2) {
      ctx.strokeStyle = `rgba(${c},${(k / tr.length) * 0.35})`;
      ctx.lineWidth = 1 + (k / tr.length) * 2;
      ctx.beginPath(); ctx.moveTo(tr[k - 2], tr[k - 1]); ctx.lineTo(tr[k], tr[k + 1]); ctx.stroke();
    }
  }
  for (const t of world.tasks) {
    const s = 13;
    const conflict = t.workers > 1;
    const held = t.holder >= 0;
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = t.done ? `rgba(110,231,183,${1 - t.done})` : conflict ? "rgba(251,113,133,0.95)" : held ? "rgba(110,231,183,0.85)" : "rgba(255,255,255,0.28)";
    ctx.fillStyle = conflict ? "rgba(251,113,133,0.18)" : held ? "rgba(110,231,183,0.08)" : "rgba(255,255,255,0.03)";
    const g = t.done ? 1 + t.done * 2 : 1;
    ctx.beginPath();
    ctx.roundRect(t.x - (s * g) / 2, t.y - (s * g) / 2, s * g, s * g, 4);
    ctx.fill();
    ctx.stroke();
    if (t.p > 0 && !t.done) {
      ctx.fillStyle = conflict ? "rgba(251,113,133,0.9)" : "rgba(110,231,183,0.9)";
      ctx.fillRect(t.x - s / 2, t.y + s / 2 + 3, s * Math.min(1, t.p), 2);
    }
  }
  if (world.order) {
    ctx.lineWidth = 1;
    for (let i = 0; i < world.agents.length; i++) {
      const a = world.agents[i];
      const t = a.task >= 0 ? world.tasks[a.task] : null;
      if (!t || t.holder !== i) continue;
      ctx.strokeStyle = "rgba(110,231,183,0.18)";
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(t.x, t.y);
      ctx.stroke();
    }
  }
  ctx.globalCompositeOperation = "lighter";
  for (const a of world.agents) {
    const c = HUES[a.hue];
    const grd = ctx.createRadialGradient(a.x, a.y, 0, a.x, a.y, 13);
    grd.addColorStop(0, `rgba(${c},0.95)`);
    grd.addColorStop(1, `rgba(${c},0)`);
    ctx.fillStyle = grd;
    ctx.fillRect(a.x - 13, a.y - 13, 26, 26);
  }
  ctx.globalCompositeOperation = "source-over";
}
