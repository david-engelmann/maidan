import { AnimatePresence, motion, useReducedMotion, type PanInfo } from "motion/react";
import { useEffect, useReducer, useRef, useState } from "react";
import { AGENTS, LEVELS, fresh, step, type Action, type Game, type Task } from "./game";
import { Join } from "./join";

const GITHUB = "https://github.com/david-engelmann/maidan";
const pop = { type: "spring" as const, stiffness: 520, damping: 30 };

export default function App() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const openIf = () => {
      if (location.hash === "#join") setOpen(true);
    };
    openIf();
    window.addEventListener("hashchange", openIf);
    return () => window.removeEventListener("hashchange", openIf);
  }, []);
  const join = () => setOpen(true);
  return (
    <>
      <header className="top">
        <a className="brand" href="#play">
          <span className="brand-mark" aria-hidden="true" />
          Maidan
        </a>
        <a className="top-link" href={GITHUB}>
          ★ GitHub
        </a>
        <button type="button" className="top-join" onClick={join}>
          Join
        </button>
      </header>
      <main>
        <Play onJoin={join} />
        <After onJoin={join} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function useTick(running: boolean, dispatch: (a: Action) => void) {
  useEffect(() => {
    if (!running) return;
    let last = performance.now();
    let raf = 0;
    const loop = (now: number) => {
      dispatch({ type: "tick", dt: Math.min(0.05, (now - last) / 1000) });
      last = now;
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [running, dispatch]);
}

type Ghost = { x: number; y: number; down: boolean; n: number } | null;

function center(sel: string, box: HTMLElement | null) {
  const el = document.querySelector(sel);
  if (!el || !box) return null;
  const r = el.getBoundingClientRect();
  const b = box.getBoundingClientRect();
  return { x: r.left + r.width / 2 - b.left, y: r.top + r.height / 2 - b.top };
}

function botMove(g: Game): { from: string; to: string; action: Action } | null {
  const lvl = LEVELS[g.level];
  const done = g.tasks.find((t) => t.where === "agent" && t.progress >= 1);
  if (done) {
    if (!done.proof) return { from: `[data-id="${done.id}"]`, to: `[data-id="${done.id}"]`, action: { type: "proof", id: done.id } };
    if (done.review && !g.tasks.some((t) => t.where === "review"))
      return { from: `[data-id="${done.id}"]`, to: `[data-drop="review"]`, action: { type: "review", id: done.id } };
    if (!done.review) return { from: `[data-id="${done.id}"]`, to: `[data-drop="ship"]`, action: { type: "ship", id: done.id } };
  }
  const q = g.tasks.find((t) => t.where === "queue");
  if (!q) return null;
  for (let i = 0; i < lvl.agents; i++) {
    if (!g.tasks.some((t) => t.where === "agent" && t.agent === i))
      return { from: `[data-id="${q.id}"]`, to: `[data-drop="agent-${i}"]`, action: { type: "assign", id: q.id, agent: i } };
  }
  return null;
}

function Play({ onJoin }: { onJoin: () => void }) {
  const reduce = useReducedMotion() === true;
  const [g, dispatch] = useReducer(step, undefined, () => fresh(reduce ? "intro" : "attract", 0));
  const board = useRef<HTMLDivElement>(null);
  const latest = useRef(g);
  const [ghost, setGhost] = useState<Ghost>(null);
  useEffect(() => {
    latest.current = g;
  });
  useTick(g.phase === "play" || g.phase === "attract", dispatch);

  useEffect(() => {
    if (g.phase !== "attract") return;
    let gone = false;
    let timer = 0;
    const run = () => {
      if (gone) return;
      const move = botMove(latest.current);
      const box = board.current;
      if (!move || !box) {
        timer = window.setTimeout(run, 400);
        return;
      }
      const a = center(move.from, box);
      const b = center(move.to, box);
      if (!a || !b) {
        timer = window.setTimeout(run, 400);
        return;
      }
      setGhost((gh) => ({ ...a, down: false, n: (gh?.n ?? 0) + 1 }));
      timer = window.setTimeout(() => {
        setGhost((gh) => (gh ? { ...gh, down: true } : gh));
        timer = window.setTimeout(() => {
          setGhost((gh) => (gh ? { ...b, down: true, n: gh.n } : gh));
          timer = window.setTimeout(() => {
            dispatch(move.action);
            setGhost((gh) => (gh ? { ...gh, down: false } : gh));
            timer = window.setTimeout(run, 520);
          }, 420);
        }, 160);
      }, 420);
    };
    timer = window.setTimeout(run, 700);
    return () => {
      gone = true;
      window.clearTimeout(timer);
    };
  }, [g.phase]);

  const lvl = LEVELS[Math.min(g.level, LEVELS.length - 1)];
  const attract = g.phase === "attract";
  const queue = g.tasks.filter((t) => t.where === "queue");
  const inReview = g.tasks.find((t) => t.where === "review");

  function drop(task: Task, info: PanInfo) {
    const x = info.point.x - window.scrollX;
    const y = info.point.y - window.scrollY;
    const hit = document.elementsFromPoint(x, y).find((el) => el instanceof HTMLElement && el.dataset.drop);
    const target = hit instanceof HTMLElement ? hit.dataset.drop : undefined;
    if (!target) return;
    if (target.startsWith("agent-")) dispatch({ type: "assign", id: task.id, agent: Number(target.slice(6)) });
    else if (target === "review") dispatch({ type: "review", id: task.id });
    else if (target === "ship") dispatch({ type: "ship", id: task.id });
  }

  return (
    <section className="play" id="play" aria-labelledby="play-h">
      <div className="intro-copy">
        <p className="eyebrow">A 60-second game about agent swarms</p>
        <h1 id="play-h">Ship the release.</h1>
        <p className="sub">Three levels. Three rules. Then you will know what Maidan does.</p>
      </div>
      <div className={attract ? "board is-attract" : "board"} ref={board}>
        <div className="hud" aria-live="polite">
          <span className="lv">
            {attract ? "Demo" : `Level ${g.level + 1} · ${lvl.title}`}
          </span>
          <span className="goal">
            {Array.from({ length: lvl.goal }, (_, i) => (
              <span key={i} className={i < g.shipped % (attract ? lvl.goal + 1 : 99) ? "pip on" : "pip"} />
            ))}
          </span>
        </div>

        <div className="lane" aria-label="Task queue">
          <span className="lane-label">queue</span>
          <div className="lane-row">
            <AnimatePresence mode="popLayout">
              {queue.map((t) => (
                <TaskTile key={t.id} task={t} onDrop={drop} disabled={attract} dispatch={dispatch} />
              ))}
            </AnimatePresence>
          </div>
        </div>

        <div className="agents">
          {AGENTS.slice(0, lvl.agents).map((name, i) => {
            const held = g.tasks.find((t) => t.where === "agent" && t.agent === i);
            const hit = g.toast?.target === `agent-${i}`;
            return (
              <motion.div
                key={name}
                className={held ? "agent busy" : "agent"}
                data-drop={`agent-${i}`}
                layout
                initial={{ scale: 0.6, opacity: 0 }}
                animate={hit ? { scale: 1, opacity: 1, x: [0, -8, 7, -4, 0] } : { scale: 1, opacity: 1, x: 0 }}
                transition={hit ? { duration: 0.4 } : pop}
                data-toast={hit ? g.toast?.n : undefined}
              >
                <span className="agent-face" data-drop={`agent-${i}`}>
                  <svg viewBox="0 0 64 64" className="agent-ring" aria-hidden="true">
                    <circle cx="32" cy="32" r="29" className="ar-bg" />
                    <circle
                      cx="32"
                      cy="32"
                      r="29"
                      className="ar-fg"
                      pathLength={1}
                      strokeDasharray={`${held ? held.progress : 0} 1`}
                    />
                  </svg>
                  <span className="agent-name">{name}</span>
                </span>
                <span className="agent-hold">
                  <AnimatePresence mode="popLayout">
                    {held ? <TaskTile key={held.id} task={held} onDrop={drop} disabled={attract} dispatch={dispatch} /> : null}
                  </AnimatePresence>
                </span>
              </motion.div>
            );
          })}
        </div>

        <div className="docks">
          <motion.div
            className={g.level >= 1 ? "dock review" : "dock review off"}
            data-drop="review"
            animate={g.toast?.target === "review" ? { x: [0, -8, 7, -4, 0] } : { x: 0 }}
            transition={{ duration: 0.4 }}
            data-toast={g.toast?.target === "review" ? g.toast.n : undefined}
          >
            <span className="dock-icon" data-drop="review" aria-hidden="true">
              ◉
            </span>
            <span data-drop="review">{inReview ? `Reviewing ${inReview.name}` : "Review"}</span>
            {inReview ? (
              <motion.span className="review-bar" initial={{ scaleX: 0 }} animate={{ scaleX: 1 }} transition={{ duration: 1.1, ease: "linear" }} />
            ) : null}
          </motion.div>
          <motion.div
            className="dock ship"
            data-drop="ship"
            animate={g.toast?.target === "ship" ? { x: [0, -8, 7, -4, 0] } : { x: 0 }}
            transition={{ duration: 0.4 }}
            data-toast={g.toast?.target === "ship" ? g.toast.n : undefined}
          >
            <span className="dock-icon" data-drop="ship" aria-hidden="true">
              ▲
            </span>
            <span data-drop="ship">Ship · {g.total}</span>
            {reduce ? null : <Burst n={g.burst} />}
          </motion.div>
        </div>

        <AnimatePresence>
          {g.toast ? (
            <motion.p
              key={g.toast.n}
              className="toast"
              role="status"
              initial={{ y: 16, opacity: 0, scale: 0.96 }}
              animate={{ y: 0, opacity: 1, scale: 1 }}
              exit={{ y: -10, opacity: 0 }}
              transition={pop}
            >
              {g.toast.text}
            </motion.p>
          ) : null}
        </AnimatePresence>

        {ghost && attract ? (
          <motion.span
            className={ghost.down ? "bot-cursor down" : "bot-cursor"}
            aria-hidden="true"
            initial={false}
            animate={{ x: ghost.x - 22, y: ghost.y - 22, scale: ghost.down ? 0.82 : 1 }}
            transition={{ type: "spring", stiffness: 210, damping: 24 }}
          />
        ) : null}

        <AnimatePresence>
          {g.phase !== "play" ? (
            <Overlay key={`${g.phase}-${g.level}`} g={g} dispatch={dispatch} onJoin={onJoin} />
          ) : null}
        </AnimatePresence>
      </div>
    </section>
  );
}

function TaskTile({
  task,
  onDrop,
  disabled,
  dispatch,
}: {
  task: Task;
  onDrop: (t: Task, info: PanInfo) => void;
  disabled: boolean;
  dispatch: (a: Action) => void;
}) {
  const done = task.where === "agent" && task.progress >= 1;
  const moved = useRef(0);
  return (
    <motion.button
      type="button"
      layout
      layoutId={`task-${task.id}`}
      data-id={task.id}
      className={`tile${task.review ? " needs-review" : ""}${done ? " done" : ""}${done && !task.proof ? " no-proof" : ""}`}
      initial={{ y: -40, opacity: 0, scale: 0.6 }}
      animate={{ y: 0, opacity: 1, scale: 1 }}
      exit={{ scale: 0.2, opacity: 0, y: 30, transition: { duration: 0.25 } }}
      transition={pop}
      drag={!disabled}
      dragSnapToOrigin
      dragElastic={0.9}
      whileDrag={{ scale: 1.18, rotate: -4, zIndex: 30 }}
      whileTap={{ scale: 1.08 }}
      onDragStart={() => {
        moved.current = performance.now();
      }}
      onDragEnd={(_e, info) => {
        moved.current = performance.now();
        onDrop(task, info);
      }}
      onClick={() => {
        if (disabled || performance.now() - moved.current < 350) return;
        if (done && !task.proof) dispatch({ type: "proof", id: task.id });
      }}
      aria-label={`${task.name}${task.review ? ", needs review" : ""}${done ? ", finished" : ""}${done && !task.proof ? ", tap to attach evidence" : ""}`}
    >
      <span className="tile-name">{task.name}</span>
      <span className="tile-tags">
        {task.review ? <span className="tag eye">◉</span> : null}
        {done ? <span className={task.proof ? "tag proof on" : "tag proof"}>{task.proof ? "✓" : "?"}</span> : null}
      </span>
    </motion.button>
  );
}

function Burst({ n }: { n: number }) {
  if (n === 0) return null;
  return (
    <span className="burst" key={n} aria-hidden="true">
      {Array.from({ length: 12 }, (_, i) => {
        const a = (i / 12) * Math.PI * 2;
        return (
          <motion.span
            key={i}
            className="spark"
            initial={{ x: 0, y: 0, opacity: 1, scale: 1 }}
            animate={{ x: Math.cos(a) * 54, y: Math.sin(a) * 40 - 12, opacity: 0, scale: 0.3 }}
            transition={{ duration: 0.7, ease: [0.2, 0.8, 0.3, 1] }}
          />
        );
      })}
    </span>
  );
}

function Overlay({ g, dispatch, onJoin }: { g: Game; dispatch: (a: Action) => void; onJoin: () => void }) {
  const lvl = LEVELS[Math.min(g.level, LEVELS.length - 1)];
  let body;
  if (g.phase === "attract") {
    body = (
      <motion.button type="button" className="play-btn" onClick={() => dispatch({ type: "start" })} whileTap={{ scale: 0.95 }}>
        <span className="play-tri" aria-hidden="true" /> Play · 60 seconds
      </motion.button>
    );
  } else if (g.phase === "intro") {
    body = (
      <div className="card">
        <p className="card-k">Level {g.level + 1} of 3</p>
        <h2>{lvl.title}</h2>
        <p>{lvl.rule}</p>
        <button type="button" className="go" onClick={() => dispatch({ type: "next" })}>
          Go
        </button>
      </div>
    );
  } else if (g.phase === "clear") {
    body = (
      <div className="card">
        <p className="card-k">Level {g.level + 1} cleared</p>
        <h2>Nice.</h2>
        <p>
          {g.level === 0
            ? "In Maidan that's a leased claim: one holder per task, released if the agent goes quiet."
            : "In Maidan that's a review gate: work waits for approval, no matter how sure the agent is."}
        </p>
        <button type="button" className="go" onClick={() => dispatch({ type: "next" })}>
          Next level
        </button>
      </div>
    );
  } else {
    body = (
      <div className="card score">
        <p className="card-k">Release shipped in {Math.round(g.time)}s</p>
        <h2>You coordinated {g.claims} claims across 4 agents.</h2>
        <p>
          {g.bounces === 0 ? "Zero bounces. Suspiciously perfect." : `${g.bounces} bounced, and every bounce was Maidan saying no.`} Maidan does
          this for real, for a hundred agents at once.
        </p>
        <div className="score-ctas">
          <button type="button" className="go" onClick={onJoin}>
            Join the waitlist
          </button>
          <a className="go ghost" href={GITHUB}>
            ★ Star on GitHub
          </a>
        </div>
        <button type="button" className="again" onClick={() => dispatch({ type: "start" })}>
          Play again
        </button>
      </div>
    );
  }
  return (
    <motion.div
      className={g.phase === "attract" ? "overlay attract" : "overlay"}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
    >
      <motion.div initial={{ y: 24, scale: 0.96 }} animate={{ y: 0, scale: 1 }} transition={pop}>
        {body}
      </motion.div>
    </motion.div>
  );
}

const RULES = [
  ["One holder", "Every task is a leased claim. Exactly one agent holds it, and the lease lapses if the agent goes quiet, so the work returns to the queue."],
  ["Footprints", "Agents declare the files they will touch. Two claims with overlapping footprints can't both be held, so nobody edits the same thing."],
  ["Review gates", "Work that needs a person waits at a gate until it's approved. Agents can't talk their way past it."],
  ["Evidence on close", "A task closes with its proof attached: tests, diffs, logs. Anyone can audit it later."],
] as const;

function After({ onJoin }: { onJoin: () => void }) {
  return (
    <section className="after" aria-labelledby="after-h">
      <h2 id="after-h">That game is the whole product.</h2>
      <p className="after-sub">Maidan is an open-source coordination layer for AI agent swarms. The rules you just played, enforced by a server.</p>
      <ol className="rules">
        {RULES.map(([t, d], i) => (
          <motion.li
            key={t}
            initial={{ opacity: 0, y: 24 }}
            whileInView={{ opacity: 1, y: 0 }}
            viewport={{ once: true, margin: "-10%" }}
            transition={{ ...pop, delay: i * 0.06 }}
          >
            <span className="rule-n">{i + 1}</span>
            <h3>{t}</h3>
            <p>{d}</p>
          </motion.li>
        ))}
      </ol>
      <p className="speaks">
        Speaks <b>MCP</b>, <b>A2A</b>, <b>REST</b> and <b>Python</b>. Self-host with <code>docker compose up</code>.
      </p>
      <div className="score-ctas">
        <button type="button" className="go" onClick={onJoin}>
          Join the waitlist
        </button>
        <a className="go ghost" href={GITHUB}>
          ★ Star on GitHub
        </a>
      </div>
    </section>
  );
}
