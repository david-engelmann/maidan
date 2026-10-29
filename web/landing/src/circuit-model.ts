// The circuit creator's model: a pipeline of stations that the visitor staffs with agents.
// Mutable by design; the canvas reads it every frame and React only hears about events.

export type Role = "planner" | "coder" | "reviewer" | "human";
export type Rule = "leases" | "reviewer" | "human";
export type StationId = "queue" | "plan" | "build" | "review" | "approve" | "main";
export type Tone = "bad" | "ok" | "info";

export const STATIONS: { id: StationId; label: string; role: Role | null; sub: string }[] = [
  { id: "queue", label: "Queue", role: null, sub: "new tasks" },
  { id: "plan", label: "Plan", role: "planner", sub: "splits the work" },
  { id: "build", label: "Build", role: "coder", sub: "holds a lease" },
  { id: "review", label: "Review", role: "reviewer", sub: "runs the tests" },
  { id: "approve", label: "Sign-off", role: "human", sub: "a person says yes" },
  { id: "main", label: "main", role: null, sub: "with evidence" },
];

export const ROLES: { id: Role; label: string; max: number; names: string[] }[] = [
  { id: "planner", label: "Planner", max: 2, names: ["Pia", "Pax"] },
  { id: "coder", label: "Coder", max: 4, names: ["Ada", "Bo", "Cy", "Dee"] },
  { id: "reviewer", label: "Reviewer", max: 2, names: ["Rae", "Rex"] },
  { id: "human", label: "Human", max: 1, names: ["You"] },
];

const TASKS = ["add-cache", "fix-login", "rate-limit", "dark-mode", "csv-export", "retry-jobs", "split-api", "trim-logs", "cold-start", "bump-deps"];

export interface Worker { id: number; role: Role; name: string; job: number | null; t: number; dur: number }
export interface Job { id: number; name: string; at: StationId; from: StationId | null; move: number; held: number[]; bad: boolean; done: number; skipped: boolean }
export interface Toast { id: number; text: string; tone: Tone }
export interface Circ {
  workers: Worker[];
  jobs: Job[];
  rules: Record<Rule, boolean>;
  spawn: number;
  nextId: number;
  taskN: number;
  shipped: number;
  collisions: number;
  broken: number;
  unsigned: number;
  caught: number;
  blockedAt: number;
  t: number;
  emit: (t: Toast | null) => void;
}

export const ORDER: StationId[] = STATIONS.map((s) => s.id);
export const next = (s: StationId): StationId => ORDER[Math.min(ORDER.length - 1, ORDER.indexOf(s) + 1)];

export function makeCirc(emit: (t: Toast | null) => void): Circ {
  return {
    workers: [],
    jobs: [],
    rules: { leases: false, reviewer: false, human: false },
    spawn: 0.2,
    nextId: 1,
    taskN: 0,
    shipped: 0,
    collisions: 0,
    broken: 0,
    unsigned: 0,
    caught: 0,
    blockedAt: -99,
    t: 0,
    emit,
  };
}

export function count(c: Circ, role: Role) {
  return c.workers.filter((w) => w.role === role).length;
}

export function addWorker(c: Circ, role: Role): Worker | null {
  const def = ROLES.find((r) => r.id === role);
  if (!def) return null;
  const taken = new Set(c.workers.map((w) => w.name));
  const name = def.names.find((n) => !taken.has(n));
  if (!name || count(c, role) >= def.max) return null;
  const w: Worker = { id: c.nextId++, role, name, job: null, t: 0, dur: 0 };
  c.workers.push(w);
  return w;
}

export function removeWorker(c: Circ, id: number) {
  c.workers = c.workers.filter((w) => w.id !== id);
  for (const j of c.jobs) j.held = j.held.filter((h) => h !== id);
}

const say = (c: Circ, text: string, tone: Tone) => c.emit({ id: c.nextId++, text, tone });

function moveTo(j: Job, to: StationId) {
  j.from = j.at;
  j.at = to;
  j.move = 0;
  j.held = [];
}

const ready = (j: Job) => j.move >= 1 && j.done === 0;
const name = (c: Circ, id: number) => c.workers.find((w) => w.id === id)?.name ?? "someone";

const STAGE: Partial<Record<StationId, { role: Role; dur: number }>> = {
  plan: { role: "planner", dur: 1.0 },
  build: { role: "coder", dur: 2.0 },
  review: { role: "reviewer", dur: 1.1 },
  approve: { role: "human", dur: 0.8 },
};

export function tick(c: Circ, dt: number): boolean {
  let changed = false;
  c.t += dt;
  c.spawn -= dt;
  if (c.spawn <= 0) {
    c.spawn = 1.5;
    if (c.jobs.filter((j) => j.at === "queue").length < 3 && c.jobs.length < 12) {
      const n = TASKS[c.taskN++ % TASKS.length];
      c.jobs.push({ id: c.nextId++, name: n, at: "queue", from: null, move: 1, held: [], bad: c.taskN % 4 === 2, done: 0, skipped: false });
      changed = true;
    }
  }
  for (const j of c.jobs) if (j.move < 1) j.move = Math.min(1, j.move + dt * 1.8);

  // Bypass stations whose rule was taken away.
  for (const j of c.jobs) {
    if (!ready(j)) continue;
    if (j.at === "review" && c.rules.reviewer) {
      j.skipped = true;
      moveTo(j, "approve");
      changed = true;
    } else if (j.at === "approve" && c.rules.human) {
      c.unsigned++;
      if (c.unsigned % 2 === 1) say(c, `${j.name} reached main with no sign-off. Nobody said yes.`, "bad");
      moveTo(j, "main");
      changed = true;
    }
  }

  // Idle workers claim waiting jobs at their station.
  for (const w of c.workers) {
    if (w.job !== null) continue;
    const st = (Object.keys(STAGE) as StationId[]).find((s) => STAGE[s]?.role === w.role);
    if (!st) continue;
    const from: StationId = st === "plan" ? "queue" : st;
    const free = c.jobs.find((j) => j.at === from && ready(j) && j.held.length === 0);
    const held = c.jobs.find((j) => j.at === from && ready(j) && j.held.length === 1);
    if (st === "build" && held && (!free || Math.random() < 0.5)) {
      if (c.rules.leases) {
        held.held.push(w.id);
        w.job = held.id;
        w.t = 0;
        w.dur = STAGE.build?.dur ?? 2;
        changed = true;
        continue;
      }
      if (!free && c.t - c.blockedAt > 7) {
        c.blockedAt = c.t;
        say(c, `${w.name} reached for ${held.name}. ${name(c, held.held[0])} already holds it. One holder, one task.`, "info");
      }
    }
    if (free) {
      free.held.push(w.id);
      w.job = free.id;
      w.t = 0;
      w.dur = (STAGE[st]?.dur ?? 1) * (0.85 + Math.random() * 0.3);
      changed = true;
    }
  }

  // Work progresses; finished work moves on.
  for (const w of c.workers) {
    if (w.job === null) continue;
    const j = c.jobs.find((x) => x.id === w.job);
    if (!j) {
      w.job = null;
      continue;
    }
    w.t += dt;
    if (w.t < w.dur) continue;
    const others = j.held.filter((h) => h !== w.id);
    w.job = null;
    changed = true;
    if (w.role === "coder" && others.length > 0) {
      // Two holders: the other copy is thrown away when the first one finishes.
      for (const o of others) {
        const ow = c.workers.find((x) => x.id === o);
        if (ow) ow.job = null;
      }
      c.collisions++;
      say(c, `${w.name} and ${name(c, others[0])} both built ${j.name}. One copy goes in the bin.`, "bad");
    }
    if (w.role === "planner") moveTo(j, "build");
    else if (w.role === "coder") moveTo(j, "review");
    else if (w.role === "reviewer") {
      if (j.bad) {
        j.bad = false;
        c.caught++;
        say(c, `${w.name} sent ${j.name} back: 2 tests failed. main stays green.`, "ok");
        moveTo(j, "build");
      } else moveTo(j, "approve");
    } else if (w.role === "human") moveTo(j, "main");
  }

  // Arrivals at main.
  for (const j of c.jobs) {
    if (j.at !== "main" || j.move < 1) continue;
    if (j.done === 0) {
      changed = true;
      if (j.bad && j.skipped) {
        c.broken++;
        say(c, `${j.name} merged untested. main is red.`, "bad");
      } else c.shipped++;
    }
    j.done += dt;
  }
  const before = c.jobs.length;
  c.jobs = c.jobs.filter((j) => j.done < 1.2);
  return changed || before !== c.jobs.length;
}

export function waiting(c: Circ, s: StationId) {
  const from: StationId = s;
  return c.jobs.filter((j) => j.at === from && j.move >= 1 && j.held.length === 0 && j.done === 0).length;
}

export function yaml(c: Circ): string {
  const lines = ["# maidan.yaml · the swarm you built", "square: my-swarm", "agents:"];
  for (const r of ROLES) {
    const n = count(c, r.id);
    if (n && r.id !== "human") lines.push(`  - role: ${r.id}`, `    count: ${n}`);
  }
  if (!ROLES.some((r) => r.id !== "human" && count(c, r.id))) lines.push("  [] # drag a Planner and a Coder onto the board above");
  const h = count(c, "human");
  if (h) lines.push("people:", "  - role: approver", `    count: ${h}`);
  lines.push(
    "rules:",
    `  lease: ${c.rules.leases ? "off      # two agents can hold one task" : "{ ttl: 15s, renew: heartbeat }"}`,
    `  review_gate: ${c.rules.reviewer ? "off  # untested work can merge" : "{ approvals: 1 }"}`,
    `  sign_off: ${c.rules.human ? "off     # nobody says yes" : "required"}`,
    "  evidence: [test_log, diff, pull_request]",
  );
  return lines.join("\n");
}

export function python(c: Circ): string {
  const role = count(c, "coder") ? "coder" : count(c, "planner") ? "planner" : "reviewer";
  return [
    "from maidan import Client",
    "",
    'client = Client("http://127.0.0.1:8080", token)',
    "",
    `# one ${role} loop: claim, heartbeat, close with proof`,
    "while (claim := client.claim_next_thread(channel_id)):",
    "    for chunk in work_on(claim):",
    "        client.renew_claim(claim.id, claim.claim_lease_id, 15)",
    "    client.threads.set_result(",
    "        claim.id, {\"tests\": \"passed\", \"pr\": pr_url},",
    "        claim.claim_lease_id,",
    "    )",
  ].join("\n");
}
