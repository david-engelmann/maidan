export type Where = "queue" | "agent" | "review" | "shipped";
export type Task = {
  id: number;
  name: string;
  review: boolean;
  where: Where;
  agent: number;
  progress: number;
  proof: boolean;
};
export type Phase = "attract" | "intro" | "play" | "clear" | "score";
export type Toast = { text: string; n: number; target: string };
export type Burst = { n: number; x: number };
export type Game = {
  phase: Phase;
  level: number;
  tasks: Task[];
  nextId: number;
  spawn: number;
  shipped: number;
  total: number;
  claims: number;
  bounces: number;
  time: number;
  reviewT: number;
  toast: Toast | null;
  burst: number;
};

export const LEVELS = [
  { title: "One holder", rule: "Drag a task onto a free agent. An agent holds one task at a time.", agents: 2, goal: 4, review: 0, proof: false },
  { title: "Review gate", rule: "Tasks with an eye need a human. Drag them to Review before they ship.", agents: 3, goal: 4, review: 0.5, proof: false },
  { title: "Evidence on close", rule: "Nothing ships without proof. Tap a finished task to attach its evidence.", agents: 4, goal: 5, review: 0.34, proof: true },
] as const;

export const AGENTS = ["Ada", "Bo", "Cy", "Dee"];
const NAMES = ["fix-auth", "add-cache", "bump-deps", "flaky-test", "rate-limit", "docs-api", "migrate-db", "lint-ci", "dark-mode", "retry-sink", "csv-export", "pager"];
const WORK = 2.1;

export function fresh(phase: Phase, level = 0, carry?: Game): Game {
  return {
    phase,
    level,
    tasks: [],
    nextId: carry?.nextId ?? 1,
    spawn: 0.8,
    shipped: 0,
    total: carry?.total ?? 0,
    claims: carry?.claims ?? 0,
    bounces: carry?.bounces ?? 0,
    time: carry?.time ?? 0,
    reviewT: 0,
    toast: null,
    burst: carry?.burst ?? 0,
  };
}

export type Action =
  | { type: "tick"; dt: number }
  | { type: "assign"; id: number; agent: number }
  | { type: "review"; id: number }
  | { type: "ship"; id: number }
  | { type: "proof"; id: number }
  | { type: "miss"; id: number }
  | { type: "start" }
  | { type: "next" }
  | { type: "attract" };

function bounce(g: Game, text: string, target: string): Game {
  return { ...g, bounces: g.bounces + (g.phase === "play" ? 1 : 0), toast: { text, n: (g.toast?.n ?? 0) + 1, target } };
}

function patch(g: Game, id: number, p: Partial<Task>): Task[] {
  return g.tasks.map((t) => (t.id === id ? { ...t, ...p } : t));
}

function ship(g: Game, id: number): Game {
  const tasks = patch(g, id, { where: "shipped" });
  const shipped = g.shipped + 1;
  const total = g.total + 1;
  const lvl = LEVELS[g.level];
  const cleared = g.phase === "play" && shipped >= lvl.goal;
  return {
    ...g,
    tasks,
    shipped: g.phase === "attract" && shipped >= 99 ? 0 : shipped,
    total,
    burst: g.burst + 1,
    phase: cleared ? (g.level >= LEVELS.length - 1 ? "score" : "clear") : g.phase,
  };
}

export function step(g: Game, a: Action): Game {
  const lvl = LEVELS[Math.min(g.level, LEVELS.length - 1)];
  const live = g.phase === "play" || g.phase === "attract";
  switch (a.type) {
    case "start":
      return fresh("intro", 0);
    case "attract":
      return fresh("attract", 0);
    case "next":
      if (g.phase === "intro") return { ...g, phase: "play" };
      if (g.phase === "clear") return { ...fresh("intro", g.level + 1, g) };
      return g;
    case "tick": {
      if (!live) return g;
      let { spawn, nextId, reviewT } = g;
      let tasks = g.tasks.filter((t) => t.where !== "shipped");
      spawn -= a.dt;
      const queued = tasks.filter((t) => t.where === "queue").length;
      if (spawn <= 0 && queued < 4) {
        spawn = 2.3 - g.level * 0.35;
        const review = lvl.review > 0 && (nextId * 0.618) % 1 < lvl.review;
        tasks = [...tasks, { id: nextId, name: NAMES[nextId % NAMES.length], review, where: "queue", agent: -1, progress: 0, proof: !lvl.proof }];
        nextId += 1;
      }
      tasks = tasks.map((t) => (t.where === "agent" && t.progress < 1 ? { ...t, progress: Math.min(1, t.progress + a.dt / WORK) } : t));
      let next: Game = { ...g, tasks, spawn, nextId, time: g.phase === "play" ? g.time + a.dt : g.time };
      const inReview = tasks.find((t) => t.where === "review");
      if (inReview) {
        reviewT += a.dt;
        next = { ...next, reviewT };
        if (reviewT > 1.1) next = ship({ ...next, reviewT: 0 }, inReview.id);
      }
      return next;
    }
    case "assign": {
      const t = g.tasks.find((x) => x.id === a.id);
      if (!t || t.where !== "queue" || a.agent >= lvl.agents) return g;
      const busy = g.tasks.find((x) => x.where === "agent" && x.agent === a.agent);
      if (busy) return bounce(g, `${AGENTS[a.agent]} already holds ${busy.name}. One holder, one task.`, `agent-${a.agent}`);
      return { ...g, claims: g.claims + 1, toast: null, tasks: patch(g, a.id, { where: "agent", agent: a.agent, progress: 0 }) };
    }
    case "proof": {
      const t = g.tasks.find((x) => x.id === a.id);
      if (!t || t.where !== "agent" || t.progress < 1 || t.proof) return g;
      return { ...g, tasks: patch(g, a.id, { proof: true }) };
    }
    case "review": {
      const t = g.tasks.find((x) => x.id === a.id);
      if (!t || t.where !== "agent") return g;
      if (t.progress < 1) return bounce(g, `${t.name} isn't finished yet. Patience.`, "review");
      if (!t.review) return bounce(g, `${t.name} doesn't need review. Ship it.`, "review");
      if (g.tasks.some((x) => x.where === "review")) return bounce(g, "The reviewer is busy. One at a time.", "review");
      if (!t.proof) return bounce(g, "No evidence attached. Tap the task to add it first.", "review");
      return { ...g, toast: null, reviewT: 0, tasks: patch(g, a.id, { where: "review" }) };
    }
    case "ship": {
      const t = g.tasks.find((x) => x.id === a.id);
      if (!t || t.where !== "agent") return g;
      if (t.progress < 1) return bounce(g, `${t.name} is half done. Shipping it would be bold.`, "ship");
      if (t.review) return bounce(g, `Unreviewed! ${t.name} needs a human first.`, "ship");
      if (!t.proof) return bounce(g, `“Trust me, it works” is not evidence. Tap to attach proof.`, "ship");
      return ship({ ...g, toast: null }, a.id);
    }
    case "miss":
      return g;
  }
}
