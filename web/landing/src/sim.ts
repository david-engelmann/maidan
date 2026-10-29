// One shared world for the 3D plaza and the 2D fallback.
export type Phase = "idle" | "toKiosk" | "leasing" | "toFountain" | "review" | "toMonument" | "drop" | "home";
export type Agent = { id: number; name: string; color: string; x: number; z: number; tx: number; tz: number; phase: Phase; t: number; kiosk: number; task: string; heading: number };
export type Kiosk = { x: number; z: number; queue: string[]; label: string; color: string };

export const KIOSKS: Kiosk[] = [
  { x: -6, z: -4, queue: ["T-28"], label: "Bugs", color: "#e8643c" },
  { x: 0, z: -7, queue: ["T-29"], label: "Docs", color: "#3f7fd9" },
  { x: 6, z: -4, queue: ["T-30"], label: "Infra", color: "#2f9e6e" },
];
export const FOUNTAIN = { x: 0, z: 0.5 };
export const MONUMENT = { x: -5.5, z: 5 };
export const HOME = { x: 5.5, z: 5 };
const NAMES = ["atlas", "birch", "cedar", "dune", "ember"];
const COLORS = ["#f4f1ea", "#2b2b30", "#f2b84b", "#7cc4e8", "#e86f8a"];
const SPEED = 2.4;

let nextTask = 31;
export const world = {
  agents: NAMES.map<Agent>((name, i) => ({ id: i, name, color: COLORS[i], x: HOME.x + (i - 2) * 0.9, z: HOME.z + (i % 2) * 0.8, tx: 0, tz: 0, phase: "idle", t: 0, kiosk: 0, task: "", heading: 0 })),
  stones: 3,
  version: 0,
  log: ["3 tasks closed with evidence today"],
  autoT: 0,
};
const listeners = new Set<() => void>();
export function subscribe(f: () => void) { listeners.add(f); return () => { listeners.delete(f); }; }
export function getVersion() { return world.version; }
function emit(msg?: string) {
  if (msg) world.log = [msg, ...world.log].slice(0, 4);
  world.version++;
  listeners.forEach((f) => f());
}

export function addTask(x?: number, z?: number, label?: string) {
  let k = Math.floor(Math.random() * KIOSKS.length);
  if (x !== undefined && z !== undefined) {
    let best = Infinity;
    KIOSKS.forEach((q, i) => { const d = (q.x - x) ** 2 + (q.z - z) ** 2; if (d < best) { best = d; k = i; } });
  }
  if (KIOSKS[k].queue.length >= 4) k = KIOSKS.findIndex((q) => q.queue.length < 4);
  if (k < 0) return;
  const id = label ?? `T-${nextTask++}`;
  KIOSKS[k].queue.push(id);
  emit(`${id} posted at the ${KIOSKS[k].label} kiosk`);
}
export function addTaskAnywhere() { addTask(undefined, undefined); }

function go(a: Agent, x: number, z: number, phase: Phase) { a.tx = x; a.tz = z; a.phase = phase; }

// discrete: jump instead of walk (reduced motion).
export function step(dt: number, discrete = false) {
  world.autoT += dt;
  const queued = KIOSKS.reduce((n, k) => n + k.queue.length, 0);
  if (world.autoT > 4 && queued < 2) { world.autoT = 0; addTask(); }
  for (const a of world.agents) {
    a.t += dt;
    if (a.phase === "idle") {
      const k = KIOSKS.findIndex((q) => q.queue.length > 0);
      if (k >= 0 && a.t > 0.4 + a.id * 0.15) {
        a.task = KIOSKS[k].queue.shift() ?? "";
        a.kiosk = k;
        const off = (a.id - 2) * 0.35;
        go(a, KIOSKS[k].x + off, KIOSKS[k].z + 1.3, "toKiosk");
        emit(`${a.name} walks to claim ${a.task}`);
      }
      continue;
    }
    if (a.phase === "toKiosk" || a.phase === "toFountain" || a.phase === "toMonument" || a.phase === "home") {
      const dx = a.tx - a.x, dz = a.tz - a.z, d = Math.hypot(dx, dz);
      if (discrete || d < 0.05) {
        a.x = a.tx; a.z = a.tz; a.t = 0;
        if (a.phase === "toKiosk") { a.phase = "leasing"; emit(`${a.name} holds the lease on ${a.task}`); }
        else if (a.phase === "toFountain") { a.phase = "review"; emit(`${a.task} waits at the fountain for a second key`); }
        else if (a.phase === "toMonument") { a.phase = "drop"; }
        else a.phase = "idle";
      } else {
        const s = Math.min(d, SPEED * dt);
        a.x += (dx / d) * s; a.z += (dz / d) * s;
        a.heading = Math.atan2(dx, dz);
      }
      continue;
    }
    if (a.phase === "leasing" && a.t > 1.4) {
      const ang = a.id * 1.25;
      go(a, FOUNTAIN.x + Math.sin(ang) * 1.9, FOUNTAIN.z + Math.cos(ang) * 1.9, "toFountain");
    } else if (a.phase === "review" && a.t > 1.8) {
      emit(`ada approved ${a.task}. Both keys turned`);
      go(a, MONUMENT.x + Math.sin(a.id * 1.3) * 1.1, MONUMENT.z + Math.cos(a.id * 1.3) * 1.1 - 0.4, "toMonument");
    } else if (a.phase === "drop" && a.t > 0.7) {
      world.stones = Math.min(world.stones + 1, 40);
      emit(`${a.name} left an evidence stone for ${a.task}`);
      a.task = "";
      go(a, HOME.x + (a.id - 2) * 0.9, HOME.z + (a.id % 2) * 0.8, "home");
    }
  }
}

export function counts() {
  const by = (p: Phase[]) => world.agents.filter((a) => p.includes(a.phase)).length;
  return {
    queued: KIOSKS.reduce((n, k) => n + k.queue.length, 0),
    leased: by(["leasing", "toFountain"]),
    review: by(["review"]),
    stones: world.stones,
  };
}
export function leaseProgress(a: Agent) {
  if (a.phase === "leasing") return Math.min(1, a.t / 1.2);
  if (a.phase === "toFountain" || a.phase === "review") return 1;
  return 0;
}
