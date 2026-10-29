import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Page } from "./page";

type Role = "planner" | "coder" | "reviewer" | "human";
type Rule = "leases" | "footprints" | "review" | "approval";
const ROLES: { id: Role; name: string; hint: string; glyph: string }[] = [
  { id: "planner", name: "Planner", hint: "Splits goals into tasks", glyph: "P" },
  { id: "coder", name: "Coder", hint: "Claims a task and ships it", glyph: "C" },
  { id: "reviewer", name: "Reviewer", hint: "Checks work before merge", glyph: "R" },
  { id: "human", name: "Human", hint: "Approves what ships", glyph: "H" },
];
const RULES: { id: Rule; name: string; what: string }[] = [
  { id: "leases", name: "Leases", what: "One holder per task. No duplicate work." },
  { id: "footprints", name: "Footprints", what: "Agents declare the files they touch. No collisions." },
  { id: "review", name: "Review gate", what: "Nothing merges until a reviewer signs off." },
  { id: "approval", name: "Human approval", what: "A person approves each ship." },
];
type Member = { id: number; role: Role };
type Ledger = { id: number; t: string; text: string; tone: "ok" | "bad" | "info" };
type Day = { hour: number; shipped: number; dup: number; conflicts: number; incidents: number; log: Ledger[] };
const EMPTY_DAY: Day = { hour: 0, shipped: 0, dup: 0, conflicts: 0, incidents: 0, log: [] };
const DAY_MS = 20000;
let SEQ = 0;

function rng(seed: number) {
  let s = seed >>> 0;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

function tick(d: Day, hour: number, team: Record<Role, number>, rules: Record<Rule, boolean>, r: () => number): Day {
  const n = { ...d, hour, log: d.log };
  const clock = `${String(9 + Math.floor(hour)).padStart(2, "0")}:${String(Math.floor((hour % 1) * 60)).padStart(2, "0")}`;
  const add = (text: string, tone: Ledger["tone"]) => { n.log = [{ id: ++SEQ, t: clock, text, tone }, ...n.log].slice(0, 6); };
  const coders = team.coder;
  if (!coders) { if (r() < 0.08) add("No coders in the square. Tasks pile up.", "bad"); return n; }
  const plan = team.planner ? 1 : 0.55;
  if (r() > 0.42 * plan * Math.min(1, coders / 2 + 0.4)) return n;
  const k = Math.floor(r() * 40) + 100;
  if (!rules.leases && coders > 1 && r() < 0.35) { n.dup++; add(`Two coders both built task #${k}. One copy thrown away.`, "bad"); return n; }
  if (!rules.footprints && coders > 1 && r() < 0.22) { n.conflicts++; add(`Merge conflict on task #${k}: same files touched.`, "bad"); return n; }
  const reviewed = rules.review && team.reviewer > 0;
  const approved = rules.approval && team.human > 0;
  const bad = r() < 0.2;
  if (bad && !reviewed && !approved) { n.incidents++; n.shipped++; add(`Task #${k} shipped with a bug. Incident opened.`, "bad"); return n; }
  if (bad) { add(`${reviewed ? "Reviewer" : "Human"} caught a bug in #${k}. Sent back.`, "info"); return n; }
  n.shipped++;
  add(`Task #${k} ${reviewed ? "reviewed, " : ""}${approved ? "approved, " : ""}shipped.`, "ok");
  return n;
}

function yaml(team: Record<Role, number>, rules: Record<Rule, boolean>) {
  const lines = ["# maidan.yaml", "square: my-swarm", "agents:"];
  for (const ro of ROLES) if (team[ro.id] && ro.id !== "human") lines.push(`  - role: ${ro.id}`, `    count: ${team[ro.id]}`);
  if (team.human) lines.push("humans:", `  - role: approver`, `    count: ${team.human}`);
  lines.push("rules:", `  leases: ${rules.leases ? "{ ttl: 10m }" : "off"}`, `  footprints: ${rules.footprints ? "required" : "off"}`, `  review_gate: ${rules.review ? "{ min_reviewers: 1 }" : "off"}`, `  human_approval: ${rules.approval ? "required" : "off"}`);
  return lines.join("\n");
}
function python(team: Record<Role, number>, rules: Record<Rule, boolean>) {
  const role = team.coder ? "coder" : team.planner ? "planner" : "reviewer";
  return [
    "from maidan import Square",
    "",
    'sq = Square.connect("my-swarm")',
    `agent = sq.join(role="${role}")`,
    "",
    "for task in agent.tasks():",
    ...(rules.leases ? ["    with agent.lease(task, ttl=600):"] : ["    with agent.claim(task):  # no lease"]),
    ...(rules.footprints ? ['        agent.declare(task, files=["src/api/**"])'] : []),
    "        result = do_work(task)",
    `        agent.submit(task, result${rules.review ? ", review=True" : ""})`,
  ].join("\n");
}

function Copy({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  return (
    <button type="button" className="k-copy" aria-label={`Copy ${label}`}
      onClick={() => { void navigator.clipboard?.writeText(text).then(() => { setDone(true); window.setTimeout(() => setDone(false), 1400); }); }}>
      {done ? "Copied" : "Copy"}
    </button>
  );
}

function Configurator() {
  const reduce = useReducedMotion();
  const [members, setMembers] = useState<Member[]>([
    { id: 1, role: "planner" }, { id: 2, role: "coder" }, { id: 3, role: "coder" }, { id: 4, role: "reviewer" },
  ]);
  const [rules, setRules] = useState<Record<Rule, boolean>>({ leases: true, footprints: true, review: true, approval: false });
  const [day, setDay] = useState<Day>(EMPTY_DAY);
  const [running, setRunning] = useState(false);
  const [drag, setDrag] = useState<{ role: Role; x: number; y: number; ox: number; oy: number; moved: boolean } | null>(null);
  const [over, setOver] = useState(false);
  const square = useRef<HTMLDivElement>(null);
  const nextId = useRef(10);
  const moved = useRef(false);
  const team: Record<Role, number> = { planner: 0, coder: 0, reviewer: 0, human: 0 };
  for (const m of members) team[m.role]++;

  const add = (role: Role) => setMembers((ms) => (ms.length >= 10 ? ms : [...ms, { id: nextId.current++, role }]));
  const remove = (id: number) => setMembers((ms) => ms.filter((m) => m.id !== id));
  const inSquare = (x: number, y: number) => {
    const b = square.current?.getBoundingClientRect();
    return !!b && x >= b.left && x <= b.right && y >= b.top && y <= b.bottom;
  };

  const teamRef = useRef(team);
  const rulesRef = useRef(rules);
  useEffect(() => { teamRef.current = team; rulesRef.current = rules; });

  useEffect(() => {
    if (!running) return;
    const r = rng(7 + Math.floor(Math.random() * 1000));
    const start = performance.now();
    const id = window.setInterval(() => {
      const p = Math.min(1, (performance.now() - start) / (reduce ? 2000 : DAY_MS));
      setDay((d) => tick(d, p * 8, teamRef.current, rulesRef.current, r));
      if (p >= 1) { setRunning(false); window.clearInterval(id); }
    }, reduce ? 40 : 160);
    return () => window.clearInterval(id);
  }, [running, reduce]);

  const startDay = () => { setDay(EMPTY_DAY); setRunning(true); };
  const ruleCount = Object.values(rules).filter(Boolean).length;
  const y = yaml(team, rules);
  const py = python(team, rules);

  return (
    <div className="k-wrap">
      {drag?.moved && <div className={`k-ghost k-${drag.role}`} style={{ transform: `translate(${drag.x - 22}px, ${drag.y - 22}px)` }} aria-hidden="true">{ROLES.find((x) => x.id === drag.role)?.glyph}</div>}
      <section className="k-step" aria-labelledby="k1">
        <p className="c-eyebrow">Step 1</p>
        <h2 id="k1" className="k-h2">Who's in your square?</h2>
        <p className="k-sub">Drag agents and people in. Tap to add, tap a member to remove.</p>
        <div className="k-palette" role="list">
          {ROLES.map((ro) => (
            <button key={ro.id} type="button" role="listitem" className={`k-chip k-${ro.id}`} data-role={ro.id}
              onClick={() => { if (moved.current) { moved.current = false; return; } add(ro.id); }}
              onPointerDown={(e) => { if (e.pointerType === "mouse" && e.button !== 0) return; e.currentTarget.setPointerCapture(e.pointerId); moved.current = false; setDrag({ role: ro.id, x: e.clientX, y: e.clientY, ox: e.clientX, oy: e.clientY, moved: false }); }}
              onPointerMove={(e) => { if (!drag) return; const mv = drag.moved || Math.hypot(e.clientX - drag.ox, e.clientY - drag.oy) > 8; moved.current = mv; setDrag({ ...drag, x: e.clientX, y: e.clientY, moved: mv }); setOver(inSquare(e.clientX, e.clientY)); }}
              onPointerUp={(e) => { if (drag && moved.current && inSquare(e.clientX, e.clientY)) add(drag.role); setDrag(null); setOver(false); }}
              onPointerCancel={() => { setDrag(null); setOver(false); }}>
              <span className="k-glyph" aria-hidden="true">{ro.glyph}</span>
              <span><b>{ro.name}</b><small>{ro.hint}</small></span>
              <span className="k-plus" aria-hidden="true">+</span>
            </button>
          ))}
        </div>
        <div ref={square} className={`k-square${over ? " is-over" : ""}${running ? " is-running" : ""}`} aria-label={`Square with ${members.length} members`}>
          <div className={`k-board r-${ruleCount}`}>
            <span className="k-board-label">The square</span>
            <AnimatePresence>
              {rules.leases && <motion.span key="l" className="k-badge" initial={{ opacity: 0, scale: 0.6 }} animate={{ opacity: 1, scale: 1 }} exit={{ opacity: 0, scale: 0.6 }}>lease</motion.span>}
              {rules.footprints && <motion.span key="f" className="k-badge" initial={{ opacity: 0, scale: 0.6 }} animate={{ opacity: 1, scale: 1 }} exit={{ opacity: 0, scale: 0.6 }}>footprint</motion.span>}
              {rules.review && <motion.span key="r" className="k-badge" initial={{ opacity: 0, scale: 0.6 }} animate={{ opacity: 1, scale: 1 }} exit={{ opacity: 0, scale: 0.6 }}>review</motion.span>}
              {rules.approval && <motion.span key="a" className="k-badge" initial={{ opacity: 0, scale: 0.6 }} animate={{ opacity: 1, scale: 1 }} exit={{ opacity: 0, scale: 0.6 }}>approval</motion.span>}
            </AnimatePresence>
          </div>
          <svg className="k-links" viewBox="-100 -100 200 200" aria-hidden="true">
            {members.map((m, i) => {
              const a = (i / Math.max(members.length, 1)) * Math.PI * 2 - Math.PI / 2;
              const ok = m.role !== "coder" || rules.leases;
              return <line key={m.id} x1={Math.cos(a) * 30} y1={Math.sin(a) * 30} x2={Math.cos(a) * 78} y2={Math.sin(a) * 78}
                className={`k-link ${ok ? "is-ok" : "is-loose"}${running ? " is-flow" : ""}`} />;
            })}
          </svg>
          {members.map((m, i) => {
            const a = (i / Math.max(members.length, 1)) * Math.PI * 2 - Math.PI / 2;
            const ro = ROLES.find((x) => x.id === m.role) ?? ROLES[0];
            return (
              <motion.button key={m.id} type="button" layout={!reduce} className={`k-member k-${m.role}`} aria-label={`Remove ${ro.name}`}
                initial={reduce ? false : { opacity: 0, scale: 0.4 }} animate={{ opacity: 1, scale: 1 }}
                transition={{ type: "spring", stiffness: 380, damping: 28 }}
                style={{ left: `${50 + Math.cos(a) * 39}%`, top: `${50 + Math.sin(a) * 39}%` }}
                onClick={() => remove(m.id)}>{ro.glyph}</motion.button>
            );
          })}
          {members.length === 0 && <p className="k-empty">Drop someone in</p>}
        </div>
        <p className="k-team" aria-live="polite">{ROLES.map((ro) => `${team[ro.id]} ${ro.name.toLowerCase()}${team[ro.id] === 1 ? "" : "s"}`).join(" · ")}</p>
      </section>

      <section className="k-step" aria-labelledby="k2">
        <p className="c-eyebrow">Step 2</p>
        <h2 id="k2" className="k-h2">Pick the rules.</h2>
        <div className="k-rules">
          {RULES.map((ru) => (
            <button key={ru.id} type="button" role="switch" aria-checked={rules[ru.id]} className={`k-rule${rules[ru.id] ? " is-on" : ""}`}
              onClick={() => setRules((rs) => ({ ...rs, [ru.id]: !rs[ru.id] }))}>
              <span className="k-rule-text"><b>{ru.name}</b><small>{ru.what}</small></span>
              <span className="k-track" aria-hidden="true"><motion.span layout={!reduce} className="k-knob" /></span>
            </button>
          ))}
        </div>
      </section>

      <section className="k-step" aria-labelledby="k3">
        <p className="c-eyebrow">Step 3</p>
        <h2 id="k3" className="k-h2">Run a day.</h2>
        <p className="k-sub">Eight working hours in twenty seconds.</p>
        <div className="k-day">
          <div className="k-day-top">
            <button type="button" className="c-btn c-btn-primary" onClick={startDay} disabled={running}>{running ? "Running…" : day.hour ? "Run it again" : "Run a day"}</button>
            <span className="k-clock" aria-label="Time of day">{String(9 + Math.floor(day.hour)).padStart(2, "0")}:{String(Math.floor((day.hour % 1) * 60)).padStart(2, "0")}</span>
          </div>
          <div className="k-progress" aria-hidden="true"><span style={{ transform: `scaleX(${day.hour / 8})` }} /></div>
          <dl className="k-stats">
            <div><dt>Shipped</dt><dd>{day.shipped}</dd></div>
            <div className={day.dup ? "is-bad" : ""}><dt>Duplicate work</dt><dd>{day.dup}</dd></div>
            <div className={day.conflicts ? "is-bad" : ""}><dt>Conflicts</dt><dd>{day.conflicts}</dd></div>
            <div className={day.incidents ? "is-bad" : ""}><dt>Incidents</dt><dd>{day.incidents}</dd></div>
          </dl>
          <ol className="k-ledger" aria-live="polite">
            {day.log.length === 0 && <li className="k-idle">The ledger fills as your square works.</li>}
            <AnimatePresence initial={false}>
              {day.log.map((l) => (
                <motion.li key={l.id} className={`k-${l.tone}`}
                  initial={reduce ? false : { opacity: 0, y: -10 }} animate={{ opacity: 1, y: 0 }}>
                  <time>{l.t}</time>{l.text}
                </motion.li>
              ))}
            </AnimatePresence>
          </ol>
        </div>
      </section>

      <section className="k-step" aria-labelledby="k4" id="config">
        <p className="c-eyebrow">Your swarm</p>
        <h2 id="k4" className="k-h2">Here's your config.</h2>
        <div className="k-out">
          <figure className="k-code"><figcaption>maidan.yaml<Copy text={y} label="YAML config" /></figcaption><pre><code>{y}</code></pre></figure>
          <figure className="k-code"><figcaption>agent.py<Copy text={py} label="Python snippet" /></figcaption><pre><code>{py}</code></pre></figure>
        </div>
      </section>
    </div>
  );
}

export default function App() {
  return (
    <Page>
      <header className="k-hero">
        <p className="c-eyebrow">Open-source coordination for agent swarms</p>
        <h1 className="c-h1">Design your swarm<br />in 30 seconds.</h1>
        <p className="c-lede">Pick who's in. Pick the rules. Watch a day go by. Leave with a config you can run.</p>
      </header>
      <Configurator />
    </Page>
  );
}
