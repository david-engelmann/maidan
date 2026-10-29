// Pure simulation for the orchestra graph. Every function returns new state.

export type Kind = "queue" | "review" | "main" | "agent";
export type Scene = "home" | "lease" | "gate" | "evidence" | "swarm";

export interface Node {
  id: number;
  kind: Kind;
  name: string;
  x: number;
  y: number;
  hx: number;
  hy: number;
  pinned: boolean;
  task: Task | null;
  phase: "idle" | "claiming" | "working";
  progress: number;
  lease: number;
  beat: number;
  cool: number;
}

export interface Task {
  name: string;
  files: string[];
}

export interface Pulse {
  id: number;
  from: number;
  to: number;
  t: number;
  kind: "claim" | "review" | "proof";
  task: Task;
}

export interface Camera {
  cx: number;
  cy: number;
  s: number;
}

export interface Sim {
  nodes: Node[];
  pulses: Pulse[];
  seed: number;
  nextId: number;
  shipped: number;
  claims: number;
  reviewed: number;
  base: number;
  target: number;
  grow: number;
  cam: Camera;
  dragging: number | null;
}

export const QUEUE = 0;
export const REVIEW = 1;
export const MAIN = 2;
export const LEASE_SECONDS = 15;

const NAMES = ["Ada", "Bo", "Cy", "Dee", "Eli", "Fay", "Gus", "Hal", "Ivy", "Jo", "Kit", "Lu"];
const TASKS = ["fix-login", "add-cache", "bump-deps", "split-api", "docs-pass", "flaky-test", "rate-limit", "dark-mode", "retry-jobs", "trim-logs", "csv-export", "cold-start"];
const FILES = ["auth.ts", "cache.rs", "Cargo.toml", "api/mod.rs", "README.md", "ci.yml", "limits.rs", "theme.css", "jobs.py", "log.rs", "export.ts", "boot.rs"];

const DIRS = ["web", "server", "sdk", "cli", "infra", "docs", "worker", "api", "ui", "db", "auth"];
const POOL = 240;

function rand(seed: number): [number, number] {
  const next = (seed * 1664525 + 1013904223) % 4294967296;
  return [next / 4294967296, next];
}

export function agentHome(i: number, n: number): [number, number] {
  if (n <= 12) {
    const a = (i / n) * Math.PI * 2 - Math.PI / 2;
    return [Math.cos(a) * 150, Math.sin(a) * 150];
  }
  return SPIRAL[Math.min(i, SPIRAL.length - 1)];
}

// Sunflower spiral that leaves room for the review gate and main.
const SPIRAL: [number, number][] = (() => {
  const golden = Math.PI * (3 - Math.sqrt(5));
  const out: [number, number][] = [];
  for (let k = 6; out.length < 100; k++) {
    const r = 34 * Math.sqrt(k);
    const x = Math.cos(k * golden) * r;
    const y = Math.sin(k * golden) * r;
    if (Math.hypot(x, y - 250) > 56 && Math.hypot(x, y - 345) > 56) out.push([x, y]);
  }
  return out;
})();

function makeAgent(id: number, index: number, n: number): Node {
  const [hx, hy] = agentHome(index, n);
  return {
    id,
    kind: "agent",
    name: index < NAMES.length ? NAMES[index] : `agent-${index + 1}`,
    x: hx * 0.2,
    y: hy * 0.2,
    hx,
    hy,
    pinned: false,
    task: null,
    phase: "idle",
    progress: 0,
    lease: 0,
    beat: 0,
    cool: Math.min(index * 0.35, 0.6 + (index % 7) * 0.3),
  };
}

function fixed(id: number, kind: Kind, name: string, x: number, y: number): Node {
  return { id, kind, name, x, y, hx: x, hy: y, pinned: true, task: null, phase: "idle", progress: 0, lease: 0, beat: 0, cool: 0 };
}

export function initial(): Sim {
  const nodes: Node[] = [fixed(QUEUE, "queue", "Queue", 0, 0), fixed(REVIEW, "review", "Review gate", 0, 250), fixed(MAIN, "main", "main", 0, 345)];
  for (let i = 0; i < 12; i++) nodes.push(makeAgent(3 + i, i, 12));
  return { nodes, pulses: [], seed: 7, nextId: 15, shipped: 0, claims: 0, reviewed: 0, base: 12, target: 12, grow: 0, cam: { cx: 0, cy: 0, s: 1 }, dragging: null };
}

export function agents(s: Sim): Node[] {
  return s.nodes.filter((n) => n.kind === "agent");
}

function relayout(nodes: Node[]): Node[] {
  const list = nodes.filter((n) => n.kind === "agent");
  return nodes.map((n) => {
    if (n.kind !== "agent" || n.pinned) return n;
    const [hx, hy] = agentHome(list.indexOf(n), list.length);
    return { ...n, hx, hy };
  });
}

export function addAgent(s: Sim, x?: number, y?: number): Sim {
  const count = agents(s).length;
  if (count >= 100) return s;
  const a = makeAgent(s.nextId, count, count + 1);
  const placed = x === undefined || y === undefined ? a : { ...a, x, y, hx: x, hy: y, pinned: true };
  const nodes = relayout([...s.nodes, placed]);
  const base = s.target === s.base ? s.base + 1 : s.base;
  return { ...s, nodes, nextId: s.nextId + 1, base, target: Math.max(s.target === s.base ? base : s.target, count + 1) };
}

export function setTarget(s: Sim, scene: Scene): Sim {
  return { ...s, target: scene === "swarm" ? 100 : s.base };
}

export function moveNode(s: Sim, id: number, x: number, y: number): Sim {
  return { ...s, dragging: id, nodes: s.nodes.map((n) => (n.id === id ? { ...n, x, y, hx: x, hy: y, pinned: true } : n)) };
}

export function release(s: Sim): Sim {
  return { ...s, dragging: null };
}

function heldFiles(nodes: Node[]): Set<string> {
  const out = new Set<string>();
  for (const n of nodes) if (n.task) for (const f of n.task.files) out.add(f);
  return out;
}

export function step(s: Sim, dt: number, cam: Camera, speed: number, hold: number | null = null): Sim {
  let seed = s.seed;
  let nextId = s.nextId;
  let { shipped, claims, reviewed, grow } = s;
  let nodes = s.nodes;
  const arrivals: Pulse[] = [];

  // Grow or shrink the swarm toward its target, a few agents per frame.
  const count = nodes.filter((n) => n.kind === "agent").length;
  grow += dt;
  if (count < s.target && grow > 0.03) {
    grow = 0;
    const add = Math.min(3, s.target - count);
    const extra: Node[] = [];
    for (let i = 0; i < add; i++) extra.push(makeAgent(nextId + i, count + i, s.target));
    nextId += add;
    nodes = relayout([...nodes, ...extra]);
  } else if (count > s.target) {
    const keep = new Set(nodes.filter((n) => n.kind === "agent").slice(0, s.target).map((n) => n.id));
    nodes = relayout(nodes.filter((n) => n.kind !== "agent" || keep.has(n.id)));
  }
  const alive = new Set(nodes.map((n) => n.id));

  // Move pulses; record the ones that arrived.
  const pulses: Pulse[] = [];
  for (const p of s.pulses) {
    if (!alive.has(p.from) || !alive.has(p.to)) continue;
    const t = p.t + dt * speed * (p.kind === "claim" ? 1.6 : 1.1);
    if (t >= 1) arrivals.push(p);
    else pulses.push({ ...p, t });
  }

  const busy = heldFiles(nodes);
  nodes = nodes.map((n) => {
    const k = n.id === s.dragging ? 1 : Math.min(1, dt * 7);
    let next: Node = { ...n, x: n.x + (n.hx - n.x) * k, y: n.y + (n.hy - n.y) * k };
    if (n.kind !== "agent") return next;
    const arrived = arrivals.find((p) => p.kind === "claim" && p.to === n.id);
    if (arrived) next = { ...next, phase: "working", progress: 0, lease: 1, beat: 0 };
    if (next.phase === "working") {
      const beat = next.beat + dt * speed;
      const lease = beat > 3 ? 1 : Math.max(0, next.lease - (dt * speed) / LEASE_SECONDS);
      let r: number;
      [r, seed] = rand(seed);
      const raw = next.progress + dt * speed * (0.22 + r * 0.18);
      const progress = n.id === hold ? Math.min(0.9, raw) : raw;
      next = { ...next, beat: beat > 3 ? 0 : beat, lease, progress };
      if (progress >= 1 && next.task) {
        pulses.push({ id: nextId++, from: n.id, to: REVIEW, t: 0, kind: "review", task: next.task });
        next = { ...next, phase: "idle", task: null, progress: 0, lease: 0, cool: 0.8 };
      }
    } else if (next.phase === "idle") {
      const cool = next.cool - dt * speed;
      next = { ...next, cool };
      if (cool <= 0) {
        let r: number;
        [r, seed] = rand(seed);
        const start = Math.floor(r * POOL);
        for (let j = 0; j < POOL; j++) {
          const ti = (start + j) % POOL;
          const files = [`${DIRS[ti % DIRS.length]}/${FILES[ti % FILES.length]}`, `${DIRS[(ti + 3) % DIRS.length]}/${FILES[(ti + 5) % FILES.length]}`];
          if (files.some((f) => busy.has(f))) continue;
          for (const f of files) busy.add(f);
          const task = { name: ti < TASKS.length ? TASKS[ti] : `${TASKS[ti % TASKS.length]}-${Math.floor(ti / TASKS.length) + 1}`, files };
          pulses.push({ id: nextId++, from: QUEUE, to: n.id, t: 0, kind: "claim", task });
          claims += 1;
          next = { ...next, phase: "claiming", task };
          break;
        }
        if (next.phase === "idle") next = { ...next, cool: 0.4 };
      }
    }
    return next;
  });

  for (const p of arrivals) {
    if (p.kind === "review") {
      reviewed += 1;
      pulses.push({ id: nextId++, from: REVIEW, to: MAIN, t: -0.35, kind: "proof", task: p.task });
    } else if (p.kind === "proof") shipped += 1;
  }

  const ck = Math.min(1, dt * 3.2);
  const c = s.cam;
  const camNext = speed >= 1 ? { cx: c.cx + (cam.cx - c.cx) * ck, cy: c.cy + (cam.cy - c.cy) * ck, s: c.s + (cam.s - c.s) * ck } : cam;

  return { ...s, nodes, pulses, seed, nextId, shipped, claims, reviewed, grow, cam: camNext };
}

export function pulsePoint(s: Sim, p: Pulse): [number, number] {
  const a = s.nodes.find((n) => n.id === p.from);
  const b = s.nodes.find((n) => n.id === p.to);
  if (!a || !b) return [0, 0];
  const t = Math.max(0, p.t);
  const e = t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
  return [a.x + (b.x - a.x) * e, a.y + (b.y - a.y) * e];
}
