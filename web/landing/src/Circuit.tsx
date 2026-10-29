import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import {
  ORDER,
  ROLES,
  STATIONS,
  addWorker,
  makeCirc,
  python,
  removeWorker,
  tick,
  yaml,
  type Circ,
  type Role,
  type Rule,
  type StationId,
  type Toast,
} from "./circuit-model";
import { fitCanvas, on, roundRect, runWhileVisible } from "./loop";

const COLOR: Record<StationId, string> = {
  queue: "242,243,247",
  plan: "45,212,191",
  build: "96,165,250",
  review: "167,139,250",
  approve: "251,191,36",
  main: "74,222,128",
};

const RULES: { id: Rule; label: string; what: string }[] = [
  { id: "leases", label: "No leases", what: "Two coders can grab one task." },
  { id: "reviewer", label: "No reviewer", what: "Work skips the tests." },
  { id: "human", label: "No human", what: "Nobody signs off." },
];

interface View {
  workers: { id: number; role: Role; name: string; job: string | null }[];
  jobs: Record<StationId, string[]>;
  shipped: number;
  collisions: number;
  broken: number;
  unsigned: number;
  caught: number;
  rules: Record<Rule, boolean>;
}

function snapshot(c: Circ): View {
  const jobs = Object.fromEntries(ORDER.map((s) => [s, [] as string[]])) as Record<StationId, string[]>;
  for (const j of c.jobs) if (j.move >= 1 && j.held.length === 0 && j.at !== "main") jobs[j.at].push(j.name);
  return {
    workers: c.workers.map((w) => ({ id: w.id, role: w.role, name: w.name, job: c.jobs.find((j) => j.id === w.job)?.name ?? null })),
    jobs,
    shipped: c.shipped,
    collisions: c.collisions,
    broken: c.broken,
    unsigned: c.unsigned,
    caught: c.caught,
    rules: { ...c.rules },
  };
}

export function Circuit({ reduce, onSpec }: { reduce: boolean; onSpec: (yaml: string, py: string) => void }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const circ = useRef<Circ | null>(null);
  const [view, setView] = useState<View | null>(null);
  const board = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const cards = useRef<Partial<Record<StationId, HTMLElement | null>>>({});
  const ghost = useRef<HTMLDivElement>(null);
  const drag = useRef<{ role: Role; x: number; y: number; moved: boolean } | null>(null);
  const [dragRole, setDragRole] = useState<Role | null>(null);

  const get = useCallback((): Circ => {
    if (!circ.current)
      circ.current = makeCirc((t) => {
        if (!t) return;
        setToasts((ts) => [...ts.slice(-1), t]);
        window.setTimeout(() => setToasts((ts) => ts.filter((x) => x.id !== t.id)), 3600);
      });
    return circ.current;
  }, []);

  const publish = useCallback(() => {
    const c = get();
    setView(snapshot(c));
    onSpec(yaml(c), python(c));
  }, [get, onSpec]);

  useEffect(() => publish(), [publish]);

  const add = useCallback(
    (role: Role) => {
      if (addWorker(get(), role)) publish();
    },
    [get, publish],
  );
  const toggle = useCallback(
    (rule: Rule) => {
      const c = get();
      c.rules[rule] = !c.rules[rule];
      publish();
    },
    [get, publish],
  );

  useEffect(() => {
    const offs = [
      on("circuit:add", (r) => add(r as Role)),
      on("circuit:rule", (r) => toggle(r as Rule)),
    ];
    return () => offs.forEach((f) => f());
  }, [add, toggle]);

  // Frame loop: the model ticks, the canvas draws wires and moving work, React hears only events.
  useEffect(() => {
    const cv = canvas.current;
    const ctx = cv?.getContext("2d");
    const bd = board.current;
    if (!cv || !ctx || !bd) return;
    let since = 0;
    let dirty = false;
    let phase = 0;
    return runWhileVisible(cv, (dt) => {
      const c = get();
      if (tick(c, reduce ? dt * 0.6 : dt)) dirty = true;
      since += dt;
      if (dirty && since > 0.12) {
        since = 0;
        dirty = false;
        setView(snapshot(c));
      }
      phase += dt;
      const { w, h, dpr } = fitCanvas(cv);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const base = cv.getBoundingClientRect();
      const ctr: Partial<Record<StationId, [number, number, number, number]>> = {};
      for (const s of ORDER) {
        const el = cards.current[s];
        if (!el) continue;
        const r = el.getBoundingClientRect();
        ctr[s] = [r.left - base.left + r.width / 2, r.top - base.top + r.height / 2, r.width, r.height];
      }
      const vertical = w < 760;
      // Wires.
      for (let i = 0; i < ORDER.length - 1; i++) {
        const a = ctr[ORDER[i]];
        const b = ctr[ORDER[i + 1]];
        if (!a || !b) continue;
        const skip = (ORDER[i + 1] === "review" && c.rules.reviewer) || (ORDER[i + 1] === "approve" && c.rules.human);
        ctx.lineWidth = 2;
        ctx.setLineDash([3, 7]);
        ctx.lineDashOffset = reduce ? 0 : -phase * 28;
        ctx.strokeStyle = skip ? "rgba(244,63,94,0.55)" : `rgba(${COLOR[ORDER[i + 1]]},0.5)`;
        ctx.beginPath();
        if (vertical) {
          ctx.moveTo(a[0], a[1] + a[3] / 2);
          ctx.lineTo(b[0], b[1] - b[3] / 2);
        } else {
          ctx.moveTo(a[0] + a[2] / 2, a[1]);
          ctx.lineTo(b[0] - b[2] / 2, b[1]);
        }
        ctx.stroke();
      }
      ctx.setLineDash([]);
      // Bypass arcs around a station whose rule was taken away.
      const bypass = (s: StationId) => {
        const i = ORDER.indexOf(s);
        const a = ctr[ORDER[i - 1]];
        const b = ctr[ORDER[i + 1]];
        if (!a || !b) return;
        ctx.strokeStyle = "rgba(244,63,94,0.9)";
        ctx.lineWidth = 2;
        ctx.setLineDash([6, 6]);
        ctx.lineDashOffset = reduce ? 0 : -phase * 40;
        ctx.beginPath();
        if (vertical) {
          const x = Math.max(a[0] + a[2] / 2, b[0] + b[2] / 2) - 6;
          ctx.moveTo(a[0] + a[2] / 2 - 6, a[1]);
          ctx.bezierCurveTo(x + 26, a[1], x + 26, b[1], b[0] + b[2] / 2 - 6, b[1]);
        } else {
          const y = Math.min(a[1] - a[3] / 2, b[1] - b[3] / 2) + 6;
          ctx.moveTo(a[0], a[1] - a[3] / 2 + 6);
          ctx.bezierCurveTo(a[0], y - 34, b[0], y - 34, b[0], b[1] - b[3] / 2 + 6);
        }
        ctx.stroke();
        ctx.setLineDash([]);
      };
      if (c.rules.reviewer) bypass("review");
      if (c.rules.human) bypass("approve");
      // Work in flight.
      for (const j of c.jobs) {
        if (j.move >= 1 && j.at !== "main") continue;
        const a = j.from ? ctr[j.from] : undefined;
        const b = ctr[j.at];
        if (!b) continue;
        const t = j.move;
        const e = t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
        let x = b[0];
        let y = b[1];
        if (a && t < 1) {
          x = a[0] + (b[0] - a[0]) * e;
          y = a[1] + (b[1] - a[1]) * e;
          const skipping = j.skipped && (j.at === "approve" || j.at === "main");
          if (skipping) {
            if (vertical) x += Math.sin(Math.PI * e) * (a[2] / 2 + 18);
            else y -= Math.sin(Math.PI * e) * (a[3] / 2 + 26);
          }
        }
        const red = j.bad && j.skipped;
        const col = red ? "244,63,94" : COLOR[j.at];
        if (j.at === "main" && t >= 1) {
          const k = Math.min(1, j.done / 1.2);
          ctx.strokeStyle = `rgba(${col},${1 - k})`;
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.arc(x, y, 12 + k * 40, 0, Math.PI * 2);
          ctx.stroke();
          continue;
        }
        const g = ctx.createRadialGradient(x, y, 0, x, y, 16);
        g.addColorStop(0, `rgba(${col},0.55)`);
        g.addColorStop(1, `rgba(${col},0)`);
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(x, y, 16, 0, Math.PI * 2);
        ctx.fill();
        roundRect(ctx, x - 5, y - 5, 10, 10, 3);
        ctx.fillStyle = `rgb(${col})`;
        ctx.fill();
      }
    });
  }, [get, reduce]);

  const down = (e: RPointerEvent, role: Role) => {
    drag.current = { role, x: e.clientX, y: e.clientY, moved: false };
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
  };
  const move = (e: RPointerEvent) => {
    const d = drag.current;
    if (!d) return;
    if (!d.moved && Math.hypot(e.clientX - d.x, e.clientY - d.y) > 6) {
      d.moved = true;
      setDragRole(d.role);
    }
    if (d.moved && ghost.current) ghost.current.style.transform = `translate(${e.clientX}px, ${e.clientY}px) translate(-50%, -50%) scale(1.06)`;
  };
  const up = (e: RPointerEvent) => {
    const d = drag.current;
    drag.current = null;
    setDragRole(null);
    if (!d) return;
    if (!d.moved) return add(d.role);
    const r = board.current?.getBoundingClientRect();
    if (r && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom) add(d.role);
  };

  const c = view;
  const full = (r: Role) => (c?.workers.filter((w) => w.role === r).length ?? 0) >= (ROLES.find((x) => x.id === r)?.max ?? 0);
  const status = !c
    ? ""
    : c.workers.length === 0
      ? "Drag a Planner onto the board to start."
      : !c.workers.some((w) => w.role === "coder")
        ? "Now a Coder. Somebody has to build."
        : !c.workers.some((w) => w.role === "reviewer")
          ? "Add a Reviewer, or nothing reaches main."
          : !c.workers.some((w) => w.role === "human")
            ? "Last one: a Human to sign off."
            : "It runs. Now take one rule away and watch what breaks.";

  return (
    <section className="build" id="build" aria-labelledby="build-title">
      <div className="build-head">
        <p className="eyebrow">Circuit creator</p>
        <h2 id="build-title">Wire up a swarm.</h2>
        <p className="lede">Drag agents onto the board. Each station only moves work forward when someone holds it.</p>
      </div>
      <div className="instrument">
        <div className="board" ref={board} data-drag={dragRole ?? undefined}>
          <canvas ref={canvas} className="board-canvas" aria-hidden="true" />
          <ol className="stations" aria-label="Pipeline">
            {STATIONS.map((s) => {
              const ws = c?.workers.filter((w) => w.role === s.role) ?? [];
              const jobs = c?.jobs[s.id] ?? [];
              const off = (s.id === "review" && c?.rules.reviewer) || (s.id === "approve" && c?.rules.human);
              const needs = s.role && ws.length === 0 && !off && jobs.length > 0;
              return (
                <li
                  key={s.id}
                  className={`station st-${s.id} ${off ? "is-off" : ""} ${dragRole && dragRole === s.role ? "is-target" : ""} ${needs ? "is-stuck" : ""}`}
                  ref={(el) => {
                    cards.current[s.id] = el;
                  }}
                >
                  <div className="st-head">
                    <b>{s.label}</b>
                    <span>{off ? "bypassed" : s.id === "build" && c?.rules.leases ? "no lease" : s.sub}</span>
                  </div>
                  {s.id === "main" ? (
                    <div className="st-main">
                      <b>{c?.shipped ?? 0}</b> shipped{c?.broken ? <em> · {c.broken} broken</em> : null}
                    </div>
                  ) : (
                    <>
                      {s.role ? (
                        <div className="st-workers">
                          <AnimatePresence initial={false}>
                            {ws.map((w) => (
                              <motion.button
                                layout={!reduce}
                                key={w.id}
                                type="button"
                                className={`worker r-${w.role} ${w.job ? "is-busy" : ""}`}
                                initial={{ scale: 0.4, opacity: 0 }}
                                animate={{ scale: 1, opacity: 1 }}
                                exit={{ scale: 0.4, opacity: 0 }}
                                transition={{ type: "spring", stiffness: 520, damping: 30 }}
                                onClick={() => {
                                  if (circ.current) removeWorker(circ.current, w.id);
                                  publish();
                                }}
                                aria-label={`Remove ${w.name}, ${w.job ? `working on ${w.job}` : "idle"}`}
                              >
                                <span className="w-dot" aria-hidden="true">{w.name[0]}</span>
                                <span className="w-name">{w.name}</span>
                                <span className="w-job">{w.job ?? "idle"}</span>
                              </motion.button>
                            ))}
                          </AnimatePresence>
                          {ws.length === 0 ? <span className="st-empty">{needs ? `needs a ${ROLES.find((r) => r.id === s.role)?.label}` : "empty"}</span> : null}
                        </div>
                      ) : null}
                      <div className="st-jobs" aria-label={`${jobs.length} waiting`}>
                        {jobs.slice(0, 3).map((j, i) => (
                          <span key={j + i} className="job">
                            {j}
                          </span>
                        ))}
                        {jobs.length > 3 ? <span className="job more">+{jobs.length - 3}</span> : null}
                      </div>
                    </>
                  )}
                </li>
              );
            })}
          </ol>
          <div className="tray" role="group" aria-label="Agents to add">
            {ROLES.map((r) => (
              <button
                key={r.id}
                type="button"
                className={`chip r-${r.id}`}
                disabled={full(r.id)}
                onPointerDown={(e) => down(e, r.id)}
                onPointerMove={move}
                onPointerUp={up}
                onPointerCancel={() => {
                  drag.current = null;
                  setDragRole(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    add(r.id);
                  }
                }}
                aria-label={`Add a ${r.label}`}
              >
                <span className="chip-dot" aria-hidden="true" />
                {r.label}
              </button>
            ))}
          </div>
          <p className="cc-status" aria-live="polite">
            {status}
          </p>
        </div>
        <aside className="rules" aria-labelledby="rules-title">
          <h3 id="rules-title">Take one away. Watch what breaks.</h3>
          <div className="rule-list">
            {RULES.map((r) => (
              <button key={r.id} type="button" className={`rule ${c?.rules[r.id] ? "is-on" : ""}`} aria-pressed={c?.rules[r.id] ?? false} onClick={() => toggle(r.id)}>
                <span className="rule-box" aria-hidden="true" />
                <span>
                  <b>{r.label}</b>
                  <small>{r.what}</small>
                </span>
              </button>
            ))}
          </div>
          <dl className="score" aria-live="polite">
            <div>
              <dt>shipped</dt>
              <dd className="ok">{c?.shipped ?? 0}</dd>
            </div>
            <div>
              <dt>built twice</dt>
              <dd className={c?.collisions ? "bad" : ""}>{c?.collisions ?? 0}</dd>
            </div>
            <div>
              <dt>broke main</dt>
              <dd className={c?.broken ? "bad" : ""}>{c?.broken ?? 0}</dd>
            </div>
            <div>
              <dt>no sign-off</dt>
              <dd className={c?.unsigned ? "bad" : ""}>{c?.unsigned ?? 0}</dd>
            </div>
          </dl>
        </aside>
        <div className="toasts" aria-live="assertive">
          <AnimatePresence>
            {toasts.map((t) => (
              <motion.div
                key={t.id}
                layout={!reduce}
                className={`toast t-${t.tone}`}
                initial={{ opacity: 0, y: 14, scale: 0.96 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -8, scale: 0.98 }}
                transition={{ type: "spring", stiffness: 420, damping: 32 }}
              >
                {t.text}
              </motion.div>
            ))}
          </AnimatePresence>
        </div>
      </div>
      <div ref={ghost} className={`ghost ${dragRole ? `is-on r-${dragRole}` : ""}`} aria-hidden="true">
        <span className="chip-dot" />
        {ROLES.find((r) => r.id === dragRole)?.label}
      </div>
    </section>
  );
}
