// The room, drawn: a seeded run of forty tasks through five agents and three
// people, on a 2D canvas. The run is simulated once, up front, into timed
// segments, so every frame is a function of the run's clock alone: the loop,
// a paused frame and the reduced-motion still are one drawing at different
// times, and every visit sees the same run.
//
// Loaded on its own (roomviz.js) when the section nears the screen, so the
// first paint carries none of it.

export type RoomPhase = 0 | 1 | 2 | 3;

export interface RoomViz {
  setPhase(phase: RoomPhase): void;
  setPaused(paused: boolean): void;
  destroy(): void;
}

export interface RoomOptions {
  phase: RoomPhase;
  paused: boolean;
  reduced: boolean;
}

type Pt = { x: number; y: number };
type RGB = [number, number, number];
type MemberKind = "agent" | "person";

interface Member {
  name: string;
  kind: MemberKind;
  angle: number;
  at: Pt;
}

// Agents in the accent, people in warm white. The ring is turned half a
// step, so no member sits at the top or bottom where the event line runs,
// and the longest names sit where their labels are centred, not run sideways.
const ROSTER: [string, MemberKind, number][] = [
  ["Claude Code", "agent", -112.5],
  ["Maya", "person", -67.5],
  ["Codex", "agent", -22.5],
  ["Ari", "person", 22.5],
  ["Cursor", "agent", 67.5],
  ["Copilot", "agent", 112.5],
  ["Jun", "person", 157.5],
  ["Gemini", "agent", 202.5],
];

const polar = (r: number, a: number): Pt => ({ x: r * Math.cos(a), y: r * Math.sin(a) });

const MEMBERS: Member[] = ROSTER.map(([name, kind, deg]) => {
  const angle = (deg * Math.PI) / 180;
  return { name, kind, angle, at: polar(1, angle) };
});

// Radii in units of the members' ring: where posted work waits, and the ring
// that landed work fills, one slot per task, clockwise from the top.
const N = 40;
const QUEUE = 0.17;
const LANDED = 0.6;
const QUEUE_SPIN = 0.5;
const TRAVEL = { post: 1.0, claim: 1.15, handoff: 1.25, back: 1.0, land: 1.2 } as const;

type SegKind = "post" | "queue" | "claim" | "work" | "handoff" | "review" | "back" | "land";

interface Seg {
  kind: SegKind;
  t0: number;
  t1: number;
  /** A trip: from a, bending toward c, to b. */
  a?: Pt;
  b?: Pt;
  c?: Pt;
  /** A wait: circling member m (or the room's middle, -1) from angle phi at w rad/s. */
  m?: number;
  phi?: number;
  w?: number;
}

interface Task {
  segs: Seg[];
  holder: number;
  slot: number;
  landAt: number;
}

type EventKind = "posted" | "claimed" | "review" | "back" | "landed";

interface RoomEvent {
  t: number;
  kind: EventKind;
  hash: string;
}

interface Run {
  tasks: Task[];
  events: RoomEvent[];
  /** When the last task lands. */
  done: number;
  /** The loop: the last landing, a hold on the full ring, and the fade. */
  length: number;
}

function mulberry32(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let r = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}

function slotPoint(slot: number): Pt {
  return polar(LANDED, -Math.PI / 2 + ((slot + 0.5) / N) * Math.PI * 2);
}

// A hand-off bends toward the middle and off the straight line, so a trip
// between opposite members never runs through the room's name; a send-back
// bends the other way, so it does not retrace the hand-off.
function chordCtrl(a: Pt, b: Pt, side: number): Pt {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  return { x: mx * 0.48 - (dy / len) * 0.26 * side, y: my * 0.48 + (dx / len) * 0.26 * side };
}

function landCtrl(a: Pt, b: Pt): Pt {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const len = Math.hypot(mx, my);
  const k = len < 0.2 ? 0 : 0.92 / len;
  return { x: mx * k, y: my * k };
}

// Spokes curl the same way round, a pinwheel: claims go out one side of
// each spoke, posts come in the other.
const spokeCtrl = (angle: number, inbound: boolean): Pt => polar(0.55, angle + (inbound ? 0.45 : -0.45));

function fnv(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function simulate(): Run {
  const rand = mulberry32(20261117);
  const range = (lo: number, hi: number) => lo + (hi - lo) * rand();
  const pick = <T,>(xs: T[]): T => xs[Math.floor(rand() * xs.length)];
  const people = MEMBERS.flatMap((m, i) => (m.kind === "person" ? [i] : []));
  const agents = MEMBERS.flatMap((m, i) => (m.kind === "agent" ? [i] : []));

  const plans = [];
  let at = 0.3;
  for (let i = 0; i < N; i++) {
    const byPerson = rand() < 0.68;
    const back = rand();
    plans.push({
      postAt: at,
      poster: pick(byPerson ? people : agents),
      // Most work goes to an agent; now and then only a person will do
      wantsPerson: rand() < 0.14,
      rounds: back < 0.025 ? 3 : back < 0.12 ? 2 : 1,
      phi: rand() * Math.PI * 2,
    });
    at += 0.75 - Math.log(1 - rand()) * 1.15;
  }

  type State = "pending" | "posting" | "queued" | "claiming" | "working" | "handoff" | "reviewing" | "returning" | "landing" | "landed";
  const st = plans.map(() => ({ s: "pending" as State, until: 0, holder: -1, reviewer: -1, round: 0, queuedAt: 0, open: null as Seg | null }));
  const tasks: Task[] = plans.map(() => ({ segs: [], holder: -1, slot: -1, landAt: Infinity }));
  const busy = MEMBERS.map(() => false);
  const readyAt = MEMBERS.map(() => 0);
  // A holder keeps its task through review, so if every member held one
  // nobody would be left to review: claims stop while only three are free
  // of work, so a review can always start, and soon.
  let holders = 0;
  const raw: { t: number; kind: EventKind; task: number }[] = [];
  const release = (m: number, t: number) => {
    busy[m] = false;
    readyAt[m] = t + range(0.3, 0.9);
  };
  const free = (m: number, t: number) => !busy[m] && t >= readyAt[m];
  let slots = 0;
  let landed = 0;
  const DT = 0.05;

  for (let tick = 0; tick < 8000 && landed < N; tick++) {
    const t = tick * DT;
    plans.forEach((p, i) => {
      const s = st[i];
      const task = tasks[i];
      switch (s.s) {
        case "pending":
          if (t >= p.postAt) {
            const from = MEMBERS[p.poster];
            task.segs.push({ kind: "post", t0: t, t1: t + TRAVEL.post, a: from.at, b: polar(QUEUE, p.phi), c: spokeCtrl(from.angle, true) });
            s.s = "posting";
            s.until = t + TRAVEL.post;
          }
          break;
        case "posting":
          if (t >= s.until) {
            s.s = "queued";
            s.queuedAt = s.until;
            raw.push({ t: s.until, kind: "posted", task: i });
          }
          break;
        case "claiming":
          if (t >= s.until) {
            s.s = "working";
            const work = MEMBERS[s.holder].kind === "person" ? range(3.6, 6) : range(2.4, 4.8);
            s.open = { kind: "work", t0: s.until, t1: Infinity, m: s.holder, phi: rand() * Math.PI * 2, w: 2.3 };
            task.segs.push(s.open);
            s.until += work;
          }
          break;
        case "working":
          if (t >= s.until) {
            const pool = MEMBERS.flatMap((_, m) => (m !== s.holder && free(m, t) ? [m] : []));
            if (pool.length && s.open) {
              const r = pick(pool);
              busy[r] = true;
              s.reviewer = r;
              s.open.t1 = t;
              const a = MEMBERS[s.holder].at;
              const b = MEMBERS[r].at;
              task.segs.push({ kind: "handoff", t0: t, t1: t + TRAVEL.handoff, a, b, c: chordCtrl(a, b, 1) });
              raw.push({ t, kind: "review", task: i });
              s.s = "handoff";
              s.until = t + TRAVEL.handoff;
            }
          }
          break;
        case "handoff":
          if (t >= s.until) {
            s.s = "reviewing";
            const look = MEMBERS[s.reviewer].kind === "person" ? range(2, 3) : range(1.3, 2.3);
            s.open = { kind: "review", t0: s.until, t1: s.until + look, m: s.reviewer, phi: rand() * Math.PI * 2, w: -2.6 };
            task.segs.push(s.open);
            s.until += look;
          }
          break;
        case "reviewing":
          if (t >= s.until) {
            s.round++;
            release(s.reviewer, t);
            const a = MEMBERS[s.reviewer].at;
            if (s.round < p.rounds) {
              const b = MEMBERS[s.holder].at;
              task.segs.push({ kind: "back", t0: t, t1: t + TRAVEL.back, a, b, c: chordCtrl(a, b, -1) });
              raw.push({ t, kind: "back", task: i });
              s.s = "returning";
              s.until = t + TRAVEL.back;
            } else {
              release(s.holder, t);
              holders--;
              const b = slotPoint(slots);
              task.slot = slots++;
              task.holder = s.holder;
              task.segs.push({ kind: "land", t0: t, t1: t + TRAVEL.land, a, b, c: landCtrl(a, b) });
              s.s = "landing";
              s.until = t + TRAVEL.land;
            }
          }
          break;
        case "returning":
          if (t >= s.until) {
            s.s = "working";
            s.open = { kind: "work", t0: s.until, t1: Infinity, m: s.holder, phi: rand() * Math.PI * 2, w: 2.3 };
            task.segs.push(s.open);
            s.until += range(1.4, 2.6);
          }
          break;
        case "landing":
          if (t >= s.until) {
            s.s = "landed";
            task.landAt = s.until;
            raw.push({ t: s.until, kind: "landed", task: i });
            landed++;
          }
          break;
        default:
          break;
      }
    });
    // Claims, oldest waiting first: one holder each, never someone already busy
    const waiting = plans.flatMap((_, i) => (st[i].s === "queued" && t >= st[i].queuedAt + 0.5 ? [i] : []));
    waiting.sort((x, y) => st[x].queuedAt - st[y].queuedAt);
    for (const i of waiting) {
      const s = st[i];
      const p = plans[i];
      const waited = t - s.queuedAt;
      const pool = MEMBERS.flatMap((m, k) => {
        if (!free(k, t)) return [];
        const fits = p.wantsPerson ? m.kind === "person" || waited > 6 : m.kind === "agent" || waited > 4;
        return fits ? [k] : [];
      });
      if (!pool.length || holders >= MEMBERS.length - 3) continue;
      const m = pick(pool);
      busy[m] = true;
      holders++;
      s.holder = m;
      tasks[i].segs.push({ kind: "queue", t0: s.queuedAt, t1: t, m: -1, phi: p.phi, w: QUEUE_SPIN });
      const from = polar(QUEUE, p.phi + QUEUE_SPIN * (t - s.queuedAt));
      tasks[i].segs.push({ kind: "claim", t0: t, t1: t + TRAVEL.claim, a: from, b: MEMBERS[m].at, c: spokeCtrl(MEMBERS[m].angle, false) });
      raw.push({ t, kind: "claimed", task: i });
      s.s = "claiming";
      s.until = t + TRAVEL.claim;
    }
  }

  raw.sort((x, y) => x.t - y.t);
  let prev = "0";
  const events = raw.map((e) => {
    // Each entry's hash covers the one before it, as the log's do
    const hash = fnv(`${prev}|${e.kind}|${e.task}|${e.t.toFixed(2)}`).toString(16).padStart(8, "0").slice(0, 7);
    prev = hash;
    return { t: e.t, kind: e.kind, hash };
  });
  const done = Math.max(...tasks.map((x) => (Number.isFinite(x.landAt) ? x.landAt : 0)));
  return { tasks, events, done, length: done + 6 };
}

let RUN: Run | null = null;
const run = () => (RUN ??= simulate());

// ---------------------------------------------------------------- drawing

interface Palette {
  agent: RGB;
  person: RGB;
  task: RGB;
  claim: RGB;
  review: RGB;
  back: RGB;
  landed: RGB;
  ink: RGB;
  muted: RGB;
  faint: RGB;
  line: RGB;
  paper: RGB;
  /** Light adds up on the dark page; on paper it is laid over. */
  glow: boolean;
}

function hex(value: string, fallback: RGB): RGB {
  const v = value.trim().replace("#", "");
  if (!/^[0-9a-f]{6}$/i.test(v)) return fallback;
  return [parseInt(v.slice(0, 2), 16), parseInt(v.slice(2, 4), 16), parseInt(v.slice(4, 6), 16)];
}

function readPalette(el: Element): Palette {
  const css = getComputedStyle(el);
  const get = (name: string, fallback: RGB) => hex(css.getPropertyValue(name), fallback);
  return {
    agent: get("--rv-agent", [255, 106, 43]),
    person: get("--rv-person", [239, 229, 214]),
    task: get("--rv-task", [255, 217, 176]),
    claim: get("--rv-claim", [255, 122, 61]),
    review: get("--rv-review", [154, 165, 255]),
    back: get("--rv-back", [255, 111, 100]),
    landed: get("--rv-landed", [79, 211, 145]),
    ink: get("--rv-ink", [236, 235, 231]),
    muted: get("--rv-muted", [159, 163, 171]),
    faint: get("--rv-faint", [108, 113, 123]),
    line: get("--rv-line", [236, 240, 248]),
    paper: get("--rv-paper", [12, 13, 15]),
    glow: css.getPropertyValue("--rv-glow").trim() !== "0",
  };
}

const rgba = (c: RGB, a: number) => `rgba(${c[0]},${c[1]},${c[2]},${Math.max(0, Math.min(1, a)).toFixed(3)})`;
const mix = (a: RGB, b: RGB, k: number): RGB => [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k].map(Math.round) as RGB;
const clamp01 = (x: number) => (x < 0 ? 0 : x > 1 ? 1 : x);
const easeInOut = (x: number) => (x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2);
const easeOut = (x: number) => 1 - Math.pow(1 - x, 3);
const easeOutBack = (x: number) => 1 + 2.2 * Math.pow(x - 1, 3) + 1.2 * Math.pow(x - 1, 2);

function bez(a: Pt, c: Pt, b: Pt, s: number): Pt {
  const u = 1 - s;
  return { x: u * u * a.x + 2 * u * s * c.x + s * s * b.x, y: u * u * a.y + 2 * u * s * c.y + s * s * b.y };
}

type Cat = "claim" | "review" | "land" | "chain";
const CAT: Record<SegKind, Cat> = { post: "claim", queue: "claim", claim: "claim", work: "claim", handoff: "review", review: "review", back: "review", land: "land" };
const FOCUS: Record<RoomPhase, Cat[]> = { 0: [], 1: ["claim"], 2: ["review"], 3: ["land", "chain"] };
const DIM = 0.16;

const EVENT_WORD: Record<EventKind, string> = { posted: "posted", claimed: "claimed", review: "to review", back: "sent back", landed: "landed" };

export function mountRoom(canvas: HTMLCanvasElement, options: RoomOptions): RoomViz {
  const ctx = canvas.getContext("2d");
  if (!ctx) return { setPhase() {}, setPaused() {}, destroy() {} };
  const g = ctx;
  const R = run();
  let { phase, paused } = options;
  const { reduced } = options;
  // The first view starts mid-run, with work already moving and landing
  let clock = reduced ? R.done + 1 : 26;
  let palette = readPalette(canvas);
  const weight: Record<Cat, number> = { claim: 1, review: 1, land: 1, chain: 1 };
  const boost: Record<Cat, number> = { claim: 0, review: 0, land: 0, chain: 0 };
  const sprites = new Map<string, HTMLCanvasElement>();

  let W = 0;
  let H = 0;
  let dpr = 1;
  const geo = { cx: 0, cy: 0, r: 0, k: 1, chainY: 0, x0: 0, x1: 0, tallies: true, font: 12 };

  function layout() {
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    if (!w || !h) return false;
    dpr = Math.min(2, window.devicePixelRatio || 1);
    if (w !== W || h !== H || canvas.width !== Math.round(w * dpr)) {
      W = w;
      H = h;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    // The event line runs along the foot, its newest end stopping short of
    // the pause control, which sits at the line's end like a player's
    const chainH = 62;
    const S = Math.max(160, Math.min(W, H - chainH));
    geo.cx = W / 2;
    geo.cy = (H - chainH) / 2;
    geo.r = S * 0.39;
    geo.k = Math.max(0.5, Math.min(1.12, geo.r / 280));
    geo.chainY = H - 26;
    geo.x0 = Math.max(12, geo.cx - S / 2);
    geo.x1 = W - 16 - 32 - 16;
    geo.tallies = S >= 440;
    geo.font = Math.max(11, Math.min(13.5, 13 * (geo.r / 280)));
    return true;
  }

  const px = (p: Pt): Pt => ({ x: geo.cx + p.x * geo.r, y: geo.cy + p.y * geo.r });

  function sprite(c: RGB): HTMLCanvasElement {
    const key = c.join(",") + (palette.glow ? "g" : "p");
    let s = sprites.get(key);
    if (s) return s;
    if (sprites.size > 96) sprites.clear();
    s = document.createElement("canvas");
    s.width = s.height = 64;
    const sg = s.getContext("2d");
    if (sg) {
      const grad = sg.createRadialGradient(32, 32, 0, 32, 32, 32);
      const a = palette.glow ? 1 : 0.5;
      grad.addColorStop(0, rgba(c, a));
      grad.addColorStop(0.22, rgba(c, a * 0.5));
      grad.addColorStop(0.55, rgba(c, a * 0.12));
      grad.addColorStop(1, rgba(c, 0));
      sg.fillStyle = grad;
      sg.fillRect(0, 0, 64, 64);
    }
    sprites.set(key, s);
    return s;
  }

  function glowAt(p: Pt, c: RGB, size: number, alpha: number) {
    if (alpha <= 0.01) return;
    g.globalAlpha = Math.min(1, alpha);
    g.drawImage(sprite(c), p.x - size / 2, p.y - size / 2, size, size);
    g.globalAlpha = 1;
  }

  function curve(a: Pt, c: Pt, b: Pt, from: number, to: number, color: RGB, alpha: number, width: number) {
    if (alpha <= 0.005 || to <= from) return;
    const steps = Math.max(2, Math.ceil((to - from) * 24));
    g.beginPath();
    for (let i = 0; i <= steps; i++) {
      const p = px(bez(a, c, b, from + ((to - from) * i) / steps));
      if (i) g.lineTo(p.x, p.y);
      else g.moveTo(p.x, p.y);
    }
    g.strokeStyle = rgba(color, alpha);
    g.lineWidth = width;
    g.stroke();
  }

  // A comet's tail: the curve behind the particle, brightening toward it
  function tail(a: Pt, c: Pt, b: Pt, at: number, color: RGB, alpha: number, width: number) {
    const len = 0.4;
    const pieces = 8;
    for (let i = 0; i < pieces; i++) {
      const s0 = at - len + (len * i) / pieces;
      const s1 = at - len + (len * (i + 1)) / pieces;
      if (s1 <= 0) continue;
      const k = (i + 1) / pieces;
      curve(a, c, b, Math.max(0, s0), s1, color, alpha * k * k, width * (0.5 + 0.7 * k));
    }
  }

  function segColor(seg: Seg, at: number): RGB {
    const p = palette;
    // In eighths, so a trip's color change uses a handful of glow sprites
    // rather than a new one each frame
    const u = Math.round(at * 8) / 8;
    switch (seg.kind) {
      case "post":
      case "queue":
        return p.task;
      case "claim":
        return mix(p.task, p.claim, u);
      case "work":
        return p.claim;
      case "handoff":
        return mix(p.claim, p.review, u);
      case "review":
        return p.review;
      case "back":
        return mix(p.back, p.claim, u * u);
      case "land":
        return mix(p.review, p.landed, u);
    }
  }

  function trailColor(kind: SegKind): RGB {
    const p = palette;
    return kind === "post" ? p.task : kind === "claim" ? p.claim : kind === "handoff" ? p.review : kind === "back" ? p.back : p.landed;
  }

  function orbitPoint(seg: Seg, t: number, radius: number): Pt {
    const centre = seg.m !== undefined && seg.m >= 0 ? MEMBERS[seg.m].at : { x: 0, y: 0 };
    const angle = (seg.phi ?? 0) + (seg.w ?? 0) * (t - seg.t0);
    if (seg.m === -1) return polar(QUEUE, angle);
    // Spirals in on arrival and out on leaving, so trips meet the orbit
    const ramp = Math.min(1, (t - seg.t0) / 0.4, (seg.t1 - t) / 0.4);
    const r = radius * easeInOut(clamp01(ramp));
    return { x: centre.x + r * Math.cos(angle), y: centre.y + r * Math.sin(angle) };
  }

  function draw(t: number, still: boolean) {
    const p = palette;
    const k = geo.k;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, W, H);
    g.lineCap = "round";
    g.lineJoin = "round";
    const end = clamp01((t - (R.length - 1.4)) / 1.4);
    const keep = 1 - easeInOut(end);
    const orbitR = Math.max(10, 22 * k) / geo.r;
    const nodeR = Math.max(6, 11 * k);
    const additive = p.glow ? "lighter" : "source-over";
    const glowScale = p.glow ? 1 : 0.6;
    // Ink on paper needs more weight than light on black to read the same
    const ink = p.glow ? 1 : 1.7;

    // The room's middle: one warm light, rising from where work is posted
    const centre = px({ x: 0, y: 0 });
    const ember = g.createRadialGradient(centre.x, centre.y, 0, centre.x, centre.y, geo.r * 1.15);
    ember.addColorStop(0, rgba(p.agent, p.glow ? 0.16 : 0.1));
    ember.addColorStop(0.45, rgba(p.agent, p.glow ? 0.05 : 0.03));
    ember.addColorStop(1, rgba(p.agent, 0));
    g.fillStyle = ember;
    g.fillRect(0, 0, W, H);

    // The square: rings for the members, the landed work and the waiting room
    const ring = (r: number, alpha: number, dash: number[]) => {
      g.setLineDash(dash);
      g.beginPath();
      g.arc(centre.x, centre.y, r * geo.r, 0, Math.PI * 2);
      g.strokeStyle = rgba(p.line, alpha);
      g.lineWidth = 1;
      g.stroke();
      g.setLineDash([]);
    };
    ring(1, 0.055 * ink, []);
    ring(QUEUE, 0.08 * ink, [1, 4]);
    // The landed ring's forty places, one tick each, so it reads as filling
    const tickA = (0.12 + 0.14 * boost.land) * Math.max(weight.land, 0.5) * ink;
    g.strokeStyle = rgba(p.line, tickA);
    g.lineWidth = 1;
    g.beginPath();
    for (let i = 0; i < N; i++) {
      const a = -Math.PI / 2 + ((i + 0.5) / N) * Math.PI * 2;
      const r0 = LANDED * geo.r - Math.max(2.5, 4 * k);
      const r1 = LANDED * geo.r + Math.max(2.5, 4 * k);
      g.moveTo(centre.x + r0 * Math.cos(a), centre.y + r0 * Math.sin(a));
      g.lineTo(centre.x + r1 * Math.cos(a), centre.y + r1 * Math.sin(a));
    }
    g.stroke();
    // Spokes: the way to each member, brightened as work travels them
    for (const m of MEMBERS) {
      const a = polar(QUEUE, m.angle);
      curve(a, spokeCtrl(m.angle, false), m.at, 0, 1, p.line, (0.05 + 0.07 * boost.claim) * Math.max(weight.claim, 0.4) * ink, 1);
    }

    // Lit links: each trip lights its curve as it goes, and the light fades
    // after. The part a caption picks out keeps its light for longer, so its
    // whole pattern shows: the pinwheel of claims, the web of reviews.
    g.globalCompositeOperation = additive;
    for (const task of R.tasks) {
      for (const seg of task.segs) {
        if (!seg.a || !seg.b || !seg.c || seg.t0 > t) continue;
        const w = weight[CAT[seg.kind]];
        const color = trailColor(seg.kind);
        if (still) {
          curve(seg.a, seg.c, seg.b, 0, 1, color, 0.07 * w * keep * glowScale * ink, 1.1);
          continue;
        }
        if (t < seg.t1) {
          const u = easeInOut((t - seg.t0) / (seg.t1 - seg.t0));
          curve(seg.a, seg.c, seg.b, 0, u, color, 0.24 * w * glowScale * ink, 1.2);
          tail(seg.a, seg.c, seg.b, u, color, 0.9 * w * glowScale * Math.min(ink, 1.3), 2.2 * Math.max(0.7, k));
        } else {
          const lit = boost[CAT[seg.kind]];
          const fade = Math.exp(-(t - seg.t1) / (1.8 + 14 * lit));
          if (fade < 0.02) continue;
          curve(seg.a, seg.c, seg.b, 0, 1, color, 0.32 * fade * w * glowScale * ink, 1.2 + 0.3 * lit);
        }
      }
    }
    g.globalCompositeOperation = "source-over";

    // Landed work: a ring that fills clockwise, one bead per task
    const landW = weight.land * keep;
    const landedNow = R.tasks.filter((x) => x.landAt <= t).sort((a, b) => a.slot - b.slot);
    if (landedNow.length && landW > 0.01) {
      const last = Math.max(...landedNow.map((x) => x.slot));
      g.beginPath();
      g.arc(centre.x, centre.y, LANDED * geo.r, -Math.PI / 2 + (0.5 / N) * Math.PI * 2, -Math.PI / 2 + ((last + 0.5) / N) * Math.PI * 2);
      g.strokeStyle = rgba(p.landed, (p.glow ? 0.32 : 0.45) * landW);
      g.lineWidth = Math.max(1, 1.4 * k);
      g.stroke();
    }
    const dotR = Math.max(2.1, 3.4 * k);
    for (const task of landedNow) {
      const at = px(slotPoint(task.slot));
      const age = t - task.landAt;
      const pop = still ? 1 : easeOutBack(clamp01(age / 0.45));
      g.globalCompositeOperation = additive;
      glowAt(at, p.landed, Math.max(14, 24 * k) * (1 + 0.8 * Math.max(0, 1 - age)), (p.glow ? 0.45 : 0.25) * landW);
      g.globalCompositeOperation = "source-over";
      if (!still && age < 0.9) {
        g.beginPath();
        g.arc(at.x, at.y, dotR + 14 * k * easeOut(age / 0.9), 0, Math.PI * 2);
        g.strokeStyle = rgba(p.landed, 0.5 * (1 - age / 0.9) * landW);
        g.lineWidth = 1;
        g.stroke();
      }
      g.beginPath();
      g.arc(at.x, at.y, dotR * pop, 0, Math.PI * 2);
      g.fillStyle = rgba(p.glow ? mix(p.landed, [255, 255, 255], 0.25) : p.landed, landW);
      g.fill();
    }

    // Members, each with a lease that runs down while it holds work
    const holding = new Map<number, { seg: Seg; u: number }>();
    if (!still) {
      for (const task of R.tasks) {
        for (const seg of task.segs) {
          if ((seg.kind === "work" || seg.kind === "review") && seg.m !== undefined && seg.t0 <= t && t < seg.t1) {
            holding.set(seg.m, { seg, u: (t - seg.t0) / Math.max(0.01, (Number.isFinite(seg.t1) ? seg.t1 : t + 1) - seg.t0) });
          }
        }
      }
    }
    // A member is a core inside a track; while it holds work the track is
    // its lease, an arc that runs down until the work moves on
    const coreR = Math.max(4, nodeR * 0.64);
    MEMBERS.forEach((m, i) => {
      const at = px(m.at);
      const color = m.kind === "agent" ? p.agent : p.person;
      const held = holding.get(i);
      g.globalCompositeOperation = additive;
      glowAt(at, color, nodeR * (held ? 6.5 : 4.6), (m.kind === "agent" ? 0.55 : 0.32) * (held ? 1 : 0.7) * glowScale);
      g.globalCompositeOperation = "source-over";
      g.beginPath();
      g.arc(at.x, at.y, nodeR, 0, Math.PI * 2);
      g.strokeStyle = rgba(m.kind === "agent" ? p.agent : p.line, (m.kind === "agent" ? 0.38 : 0.26) * ink);
      g.lineWidth = 1;
      g.stroke();
      g.beginPath();
      g.arc(at.x, at.y, coreR, 0, Math.PI * 2);
      if (!p.glow && m.kind === "person") {
        // On paper a person is the light disc, ringed in ink
        g.fillStyle = "#fffdf8";
        g.fill();
        g.strokeStyle = rgba(p.person, 0.85);
        g.lineWidth = 1.5;
        g.stroke();
      } else {
        g.fillStyle = rgba(color, 1);
        g.fill();
      }
      if (held) {
        const left = 1 - clamp01(held.u);
        const cat = CAT[held.seg.kind];
        g.beginPath();
        g.arc(at.x, at.y, nodeR, -Math.PI / 2, -Math.PI / 2 + left * Math.PI * 2);
        g.strokeStyle = rgba(held.seg.kind === "review" ? p.review : p.claim, 0.95 * Math.max(weight[cat], 0.3));
        g.lineWidth = Math.max(1.6, 2 * k);
        g.stroke();
      }
    });

    // A quiet disc under the room's name, so work passing the middle reads
    // as passing under it
    const hub = g.createRadialGradient(centre.x, centre.y, 0, centre.x, centre.y, Math.max(26, 0.16 * geo.r));
    hub.addColorStop(0, rgba(p.paper, 0.72));
    hub.addColorStop(0.7, rgba(p.paper, 0.45));
    hub.addColorStop(1, rgba(p.paper, 0));
    g.fillStyle = hub;
    g.beginPath();
    g.arc(centre.x, centre.y, Math.max(26, 0.16 * geo.r), 0, Math.PI * 2);
    g.fill();

    // Tasks in flight: soft light, colored by where each one is
    if (!still) {
      const core = Math.max(1.9, 2.7 * k);
      const halo = Math.max(18, 36 * k);
      for (const task of R.tasks) {
        const seg = task.segs.find((s) => s.t0 <= t && t < s.t1);
        if (!seg) continue;
        const w = weight[CAT[seg.kind]];
        let at: Pt;
        let u = 0;
        if (seg.a && seg.b && seg.c) {
          u = easeInOut((t - seg.t0) / (seg.t1 - seg.t0));
          at = bez(seg.a, seg.c, seg.b, u);
        } else {
          at = orbitPoint(seg, t, orbitR);
        }
        const color = segColor(seg, u);
        const pt = px(at);
        g.globalCompositeOperation = additive;
        glowAt(pt, color, halo, 0.9 * w * glowScale);
        g.globalCompositeOperation = "source-over";
        g.beginPath();
        g.arc(pt.x, pt.y, core, 0, Math.PI * 2);
        g.fillStyle = rgba(p.glow ? mix(color, [255, 255, 255], 0.55) : color, Math.max(w, 0.35));
        g.fill();
      }
    }

    // Names, and what each member has landed so far
    g.textBaseline = "middle";
    const font = geo.font;
    const tallyFont = Math.max(9.5, font * 0.8);
    const tallies = new Array(MEMBERS.length).fill(0);
    for (const task of R.tasks) if (task.landAt <= t) tallies[task.holder]++;
    MEMBERS.forEach((m, i) => {
      const at = px(m.at);
      const ca = Math.cos(m.angle);
      const sa = Math.sin(m.angle);
      // Past the orbit and its light, so a task circling never crosses a name
      const off = orbitR * geo.r + Math.max(9, 12 * k);
      const x = at.x + ca * off;
      const y = at.y + sa * off;
      const lines: [string, string, RGB, number][] = [[`560 ${font}px Geist, ui-sans-serif, system-ui, sans-serif`, m.name, p.ink, 1]];
      if (geo.tallies) lines.push([`500 ${tallyFont}px ui-monospace, SFMono-Regular, Menlo, monospace`, `${tallies[i]} landed`, p.muted, keep]);
      const lh = font * 1.3;
      const blockH = lh * lines.length;
      let top: number;
      if (Math.abs(ca) < 0.5) {
        g.textAlign = "center";
        top = sa < 0 ? y - blockH : y;
      } else {
        g.textAlign = ca > 0 ? "left" : "right";
        top = y - blockH / 2;
      }
      lines.forEach(([f, text, color, alpha], j) => {
        g.font = f;
        g.fillStyle = rgba(color, alpha);
        g.fillText(text, x, top + lh * (j + 0.5));
      });
    });
    // The room's name at its middle, and how much is waiting there
    const waiting = R.tasks.filter((x) => x.segs.some((s) => s.kind === "queue" && s.t0 <= t && t < s.t1)).length;
    g.textAlign = "center";
    g.font = `500 ${Math.max(10, font * 0.86)}px ui-monospace, SFMono-Regular, Menlo, monospace`;
    g.fillStyle = rgba(p.muted, 1);
    g.fillText("#build", centre.x, centre.y - (geo.tallies ? 6 : 0));
    if (geo.tallies && !still) {
      g.font = `500 ${tallyFont}px ui-monospace, SFMono-Regular, Menlo, monospace`;
      g.fillStyle = rgba(p.faint, 1);
      g.fillText(`${waiting} waiting`, centre.x, centre.y + 9);
    }

    drawChain(t, keep);
  }

  // The record: every hand-off is a bead on one line, each tied to the one
  // before it by its hash, newest at the right
  function drawChain(t: number, keep: number) {
    const p = palette;
    const ev = R.events;
    let n = 0;
    while (n < ev.length && ev[n].t <= t) n++;
    if (!n) return;
    const w = weight.chain * keep;
    const sp = W < 480 ? 11 : 14;
    // Fractional count, so the line slides one step as each bead arrives
    let count = n;
    for (let i = Math.max(0, n - 4); i < n; i++) count -= 1 - easeInOut(clamp01((t - ev[i].t) / 0.45));
    const y = geo.chainY;
    const { x0, x1 } = geo;
    const span = x1 - x0;
    const colorOf = (kind: EventKind) => (kind === "posted" ? p.task : kind === "claimed" ? p.claim : kind === "review" ? p.review : kind === "back" ? p.back : p.landed);
    const r = (W < 480 ? 2 : 2.6) * (1 + 0.3 * boost.chain);
    let prev: Pt | null = null;
    const first = Math.max(0, n - Math.ceil(span / sp) - 3);
    for (let i = first; i < n; i++) {
      const x = x1 - (count - 1 - i) * sp;
      if (x < x0 - sp) {
        prev = { x, y };
        continue;
      }
      const edge = clamp01((x - x0) / (span * 0.28));
      const a = edge * w;
      if (prev) {
        g.beginPath();
        g.moveTo(Math.max(prev.x, x0), y);
        g.lineTo(x, y);
        g.strokeStyle = rgba(p.line, (0.22 + 0.16 * boost.chain) * a);
        g.lineWidth = 1;
        g.stroke();
      }
      const color = colorOf(ev[i].kind);
      if (i === n - 1) {
        const fresh = 1 - clamp01((t - ev[i].t) / 1.2);
        g.globalCompositeOperation = p.glow ? "lighter" : "source-over";
        glowAt({ x, y }, color, 26 + 14 * fresh, (0.55 + 0.4 * fresh) * a * (p.glow ? 1 : 0.6));
        g.globalCompositeOperation = "source-over";
      }
      g.beginPath();
      g.arc(x, y, r, 0, Math.PI * 2);
      g.fillStyle = rgba(color, a);
      g.fill();
      // Picked out, every fifth bead shows the start of its hash (the newest
      // shows its whole hash above the line)
      if (boost.chain > 0.02 && i < n - 1 && (n - 1 - i) % 5 === 0 && W >= 480) {
        g.font = `500 9.5px ui-monospace, SFMono-Regular, Menlo, monospace`;
        g.textAlign = "center";
        g.textBaseline = "top";
        g.fillStyle = rgba(p.faint, a * boost.chain);
        g.fillText(ev[i].hash.slice(0, 4), x, y + 7);
      }
      prev = { x, y };
    }
    const last = ev[n - 1];
    g.font = `500 10.5px ui-monospace, SFMono-Regular, Menlo, monospace`;
    g.textBaseline = "alphabetic";
    g.textAlign = "left";
    g.fillStyle = rgba(p.faint, w);
    g.fillText("event log", x0, y - 12);
    g.textAlign = "right";
    g.fillStyle = rgba(p.muted, w);
    g.fillText(`${EVENT_WORD[last.kind]} · ${last.hash}`, x1, y - 12);
  }

  // ------------------------------------------------------------- the clock

  let visible = false;
  let frame = 0;
  let last = 0;
  const moving = () => visible && !document.hidden && !paused && !reduced;
  const targetOf = (cat: Cat) => (phase === 0 || FOCUS[phase].includes(cat) ? 1 : DIM);
  const settled = () => (Object.keys(weight) as Cat[]).every((c) => Math.abs(weight[c] - targetOf(c)) < 0.005 && Math.abs(boost[c] - (FOCUS[phase].includes(c) ? 1 : 0)) < 0.005);

  function ease(dt: number) {
    const f = reduced ? 1 : 1 - Math.exp(-dt / 0.22);
    for (const c of Object.keys(weight) as Cat[]) {
      weight[c] += (targetOf(c) - weight[c]) * f;
      boost[c] += ((FOCUS[phase].includes(c) ? 1 : 0) - boost[c]) * f;
    }
  }

  function step(now: number) {
    frame = 0;
    const dt = Math.min(0.1, Math.max(0, (now - last) / 1000));
    last = now;
    if (moving()) {
      clock += dt;
      if (clock >= R.length) clock -= R.length;
    }
    ease(dt);
    if (W && H) draw(clock, reduced);
    if (moving() || !settled()) frame = requestAnimationFrame(step);
  }

  function kick() {
    if (frame) return;
    last = performance.now();
    frame = requestAnimationFrame(step);
  }

  const resize = new ResizeObserver(() => {
    if (layout()) {
      if (frame) return;
      draw(clock, reduced);
    }
  });
  resize.observe(canvas);
  const seen = new IntersectionObserver((entries) => {
    visible = entries.some((e) => e.isIntersecting);
    kick();
  });
  seen.observe(canvas);
  const theme = new MutationObserver(() => {
    palette = readPalette(canvas);
    sprites.clear();
    if (W && H) draw(clock, reduced);
  });
  theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  const onVisibility = () => kick();
  document.addEventListener("visibilitychange", onVisibility);
  let live = true;
  // Canvas text waits for nothing, so draw again once Geist has loaded
  document.fonts?.ready.then(() => {
    if (live && W && H) draw(clock, reduced);
  });
  ease(1);
  if (layout()) draw(clock, reduced);

  return {
    setPhase(next) {
      phase = next;
      kick();
    },
    setPaused(next) {
      paused = next;
      kick();
    },
    destroy() {
      live = false;
      cancelAnimationFrame(frame);
      frame = 0;
      resize.disconnect();
      seen.disconnect();
      theme.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
    },
  };
}
