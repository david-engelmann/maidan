// A client-side model of a Maidan square. Nothing here talks to a server.
export type Lane = "open" | "claimed" | "review" | "done";
export const LANES: { id: Lane; label: string }[] = [
  { id: "open", label: "Open" },
  { id: "claimed", label: "Leased" },
  { id: "review", label: "In review" },
  { id: "done", label: "Closed" },
];
export type Task = {
  id: number;
  title: string;
  path: string;
  lane: Lane;
  holder?: string;
  approved?: boolean;
  evidence?: string;
};
export type Agent = { name: string; hue: number };
export type World = { tasks: Task[]; agents: Agent[]; next: number };
export type Tone = "ok" | "blocked" | "info";
export type Result = { world: World; reply: string; tone: Tone; mcp: string; py: string; moved?: number };

const NAMES = ["atlas", "birch", "cedar", "delta", "ember", "fern", "gale", "harbor", "iris"];
const HUES = [158, 265, 190, 32, 330, 120, 210, 50, 290];
export const REVIEWER = "ada";

export function seed(): World {
  return {
    next: 19,
    agents: [],
    tasks: [
      { id: 12, title: "Fix the flaky login test", path: "web/auth/login.test.ts", lane: "open" },
      { id: 13, title: "Rate-limit the search API", path: "api/search.rs", lane: "open" },
      { id: 14, title: "Migration for team invites", path: "db/migrations/0042.sql", lane: "open" },
      { id: 15, title: "Self-hosting guide", path: "docs/self-host.md", lane: "open" },
      { id: 16, title: "Retry webhooks on 5xx", path: "api/hooks.rs", lane: "open" },
      { id: 17, title: "Dark mode for the board", path: "web/theme.css", lane: "open" },
      { id: 10, title: "Trace IDs in every log", path: "core/log.rs", lane: "done", evidence: "tests 31/31 · +64 −9" },
      { id: 11, title: "Ship invite emails", path: "mail/invite.rs", lane: "done", evidence: "tests 48/48 · +120 −14" },
    ],
  };
}

export const tid = (t: Task | number) => `T-${typeof t === "number" ? t : t.id}`;
const json = (v: unknown) => JSON.stringify(v);
const patch = (w: World, id: number, p: Partial<Task>): World => ({
  ...w,
  tasks: w.tasks.map((t) => (t.id === id ? { ...t, ...p } : t)),
});
const busy = (w: World) => new Set(w.tasks.filter((t) => t.lane === "claimed" || t.lane === "review").map((t) => t.holder));

export function run(w: World, raw: string): Result {
  const cmd = raw.trim().toLowerCase();
  if (!cmd) return info(w, "Type a command, or tap one below.");

  if (cmd === "reset") return { world: seed(), reply: "Fresh square. Six open tasks, no agents.", tone: "info", mcp: "// local reset", py: "m = Maidan.connect()" };

  if (cmd === "help") return info(w, "Try: spawn 3 agents · claim next · try a conflict · request review · approve · close with evidence · add task <title>");

  const spawn = cmd.match(/^(spawn|add|start)\s*(\d+)?\s*agents?/);
  if (spawn) {
    const n = Math.min(Number(spawn[2] ?? 1) || 1, NAMES.length - w.agents.length);
    if (n <= 0) return blocked(w, "The square is full for this demo.", "agents.register", "LimitReached");
    const fresh = NAMES.slice(w.agents.length, w.agents.length + n).map((name, i) => ({ name, hue: HUES[w.agents.length + i] }));
    return {
      world: { ...w, agents: [...w.agents, ...fresh] },
      reply: `${fresh.map((a) => a.name).join(", ")} joined the square. Each one gets an identity and can claim work.`,
      tone: "ok",
      mcp: `agents.register ${json({ names: fresh.map((a) => a.name) })}`,
      py: `swarm = [m.agent(n) for n in (${fresh.map((a) => `"${a.name}"`).join(", ")})]`,
    };
  }

  const add = raw.trim().match(/^(?:add|new|create)\s+task\s+(.+)/i);
  if (add) {
    const title = add[1].slice(0, 48);
    const t: Task = { id: w.next, title, path: `src/${title.toLowerCase().replace(/[^a-z]+/g, "_").slice(0, 18)}.ts`, lane: "open" };
    return {
      world: { ...w, next: w.next + 1, tasks: [t, ...w.tasks] },
      reply: `${tid(t)} is on the board. Anyone with a free hand can claim it.`,
      tone: "ok",
      mcp: `tasks.create ${json({ title })}`,
      py: `m.tasks.create(${json(title)})`,
      moved: t.id,
    };
  }

  if (/claim/.test(cmd)) {
    if (!w.agents.length) return blocked(w, "Nobody is here to claim it yet. Try “spawn 3 agents”.", "tasks.claim_next", "NoAgent");
    const task = w.tasks.find((t) => t.lane === "open");
    if (!task) return info(w, "Nothing open. Add a task first: “add task Fix the docs”.");
    const taken = busy(w);
    const agent = w.agents.find((a) => !taken.has(a.name));
    if (!agent) return blocked(w, "Every agent already holds a lease. One holder per thread, one thread per hand.", "tasks.claim_next", "AllBusy");
    return {
      world: patch(w, task.id, { lane: "claimed", holder: agent.name }),
      reply: `${agent.name} holds ${tid(task)} for 15 minutes. The lease also fences off ${task.path}.`,
      tone: "ok",
      mcp: `tasks.claim_next ${json({ agent: agent.name, lease: "15m" })} → ${json({ task: tid(task), footprint: [task.path] })}`,
      py: `lease = ${agent.name}.claim_next(lease="15m")  # ${tid(task)}`,
      moved: task.id,
    };
  }

  if (/conflict|collide|steal|edit/.test(cmd)) {
    const held = w.tasks.find((t) => t.lane === "claimed");
    if (!held) return info(w, "No footprints yet. Claim a task first, then try to collide with it.");
    const other = w.agents.find((a) => a.name !== held.holder)?.name ?? "a newcomer";
    return {
      world: w,
      reply: `${other} tried to edit ${held.path}. Refused: ${held.holder} holds that footprint until ${tid(held)} closes or the lease runs out.`,
      tone: "blocked",
      mcp: `footprints.acquire ${json({ agent: other, paths: [held.path] })} → 409 ${json({ error: "footprint_held", holder: held.holder })}`,
      py: `except FootprintHeld as e:  # held by ${held.holder}`,
      moved: held.id,
    };
  }

  if (/review/.test(cmd)) {
    const task = w.tasks.find((t) => t.lane === "claimed");
    if (!task) return info(w, "Nothing is leased. Claim a task, then ask for review.");
    return {
      world: patch(w, task.id, { lane: "review" }),
      reply: `${task.holder} asked for review on ${tid(task)}. It can't close until ${REVIEWER} (a human) approves.`,
      tone: "ok",
      mcp: `tasks.request_review ${json({ task: tid(task), reviewer: REVIEWER })}`,
      py: `${task.holder}.request_review(lease, reviewer="${REVIEWER}")`,
      moved: task.id,
    };
  }

  if (/approve|lgtm/.test(cmd)) {
    const task = w.tasks.find((t) => t.lane === "review" && !t.approved);
    if (!task) return info(w, "Nothing is waiting on a reviewer.");
    return {
      world: patch(w, task.id, { approved: true }),
      reply: `${REVIEWER} approved ${tid(task)}. The gate is open; ${task.holder} can close with evidence.`,
      tone: "ok",
      mcp: `reviews.approve ${json({ task: tid(task), by: REVIEWER })}`,
      py: `${REVIEWER}.approve("${tid(task)}")`,
      moved: task.id,
    };
  }

  if (/close|done|ship|evidence/.test(cmd)) {
    const ready = w.tasks.find((t) => t.lane === "review" && t.approved);
    if (!ready) {
      const waiting = w.tasks.find((t) => t.lane === "review" || t.lane === "claimed");
      if (!waiting) return info(w, "Nothing to close yet.");
      return blocked(w, `Gate: ${tid(waiting)} needs an approval before it can close.`, "tasks.close", "ReviewRequired", waiting.id);
    }
    const ev = `tests ${38 + ready.id}/${38 + ready.id} · +${ready.id * 7} −${ready.id}`;
    return {
      world: patch(w, ready.id, { lane: "done", evidence: ev }),
      reply: `${tid(ready)} closed with evidence attached: ${ev}. The lease is released and the footprint lifts.`,
      tone: "ok",
      mcp: `tasks.close ${json({ task: tid(ready), evidence: ["ci://run/8812", "diff://" + ready.path] })}`,
      py: `${ready.holder}.close(lease, evidence=[ci_run, diff])`,
      moved: ready.id,
    };
  }

  return info(w, `I don't know “${raw.trim().slice(0, 40)}”. Try “help”.`);
}

function info(w: World, reply: string): Result {
  return { world: w, reply, tone: "info", mcp: "", py: "" };
}
function blocked(w: World, reply: string, tool: string, err: string, moved?: number): Result {
  return { world: w, reply, tone: "blocked", mcp: `${tool} → ${json({ error: err })}`, py: `except ${err}: ...`, moved };
}
