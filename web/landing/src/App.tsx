import { AnimatePresence, motion, useReducedMotion, type PanInfo } from "motion/react";
import { useState, type ReactNode } from "react";
import { Join } from "./join";

const GH = "https://github.com/david-engelmann/maidan";
const ease = [0.16, 1, 0.3, 1] as const;
const spring = { type: "spring" as const, stiffness: 420, damping: 34, mass: 0.7 };

type State = "open" | "leased" | "review" | "done";
type Task = { id: string; title: string; state: State; agent?: string };
type Receipt = { id: string; title: string; agent: string; reviewer: string; sha: string; tests: number };

const AGENTS = ["atlas", "birch", "cedar", "dune"];
const SEED: Task[] = [
  { id: "T-21", title: "Fix the retry storm in sync", state: "open" },
  { id: "T-22", title: "Add rate limits to /claim", state: "open" },
  { id: "T-23", title: "Migrate the ledger index", state: "open" },
  { id: "T-24", title: "Write the A2A handshake doc", state: "open" },
];
const SEED_RECEIPTS: Receipt[] = [
  { id: "T-19", title: "Pin the parser version", agent: "dune", reviewer: "ada", sha: "4be1c09", tests: 61 },
  { id: "T-20", title: "Trim the log noise", agent: "birch", reviewer: "lin", sha: "a93f2d1", tests: 38 },
];
const LABEL: Record<State, string> = { open: "Open", leased: "Leased", review: "In review", done: "Closed" };

function useSwarm() {
  const [tasks, setTasks] = useState<Task[]>(SEED);
  const [receipts, setReceipts] = useState<Receipt[]>(SEED_RECEIPTS);
  const [pulse, setPulse] = useState(0);
  const [n, setN] = useState(0);
  const bump = () => setPulse((p) => p + 1);
  const claim = (id: string) => {
    setTasks((ts) => ts.map((t) => (t.id === id && t.state === "open" ? { ...t, state: "leased", agent: AGENTS[n % AGENTS.length] } : t)));
    setN((x) => x + 1);
    bump();
  };
  const review = (id: string) => { setTasks((ts) => ts.map((t) => (t.id === id && t.state === "leased" ? { ...t, state: "review" } : t))); bump(); };
  const approve = (id: string) => {
    const t = tasks.find((x) => x.id === id);
    if (!t || t.state !== "review") return;
    setTasks((ts) => ts.map((x) => (x.id === id ? { ...x, state: "done" } : x)));
    const sha = (parseInt(id.slice(2), 10) * 2654435761 >>> 0).toString(16).slice(0, 7);
    setReceipts((rs) => [...rs, { id, title: t.title, agent: t.agent ?? "atlas", reviewer: "ada", sha, tests: 40 + (parseInt(id.slice(2), 10) % 30) }]);
    bump();
  };
  const reset = () => { setTasks(SEED); setReceipts(SEED_RECEIPTS); setN(0); bump(); };
  return { tasks, receipts, pulse, claim, review, approve, reset };
}
type Swarm = ReturnType<typeof useSwarm>;

export default function App() {
  const [open, setOpen] = useState(false);
  const swarm = useSwarm();
  return (
    <>
      <header className="top">
        <a className="brand" href="#top">Maidan</a>
        <nav className="nav" aria-label="Sections">
          <a href="#gestures">Gestures</a>
          <a href="#receipts">Receipts</a>
          <a href="#speak">Speak</a>
        </nav>
        <a className="gh" href={GH}>GitHub ↗</a>
        <button type="button" className="join-btn" onClick={() => setOpen(true)}>Get an invite</button>
      </header>
      <main id="top">
        <Hero swarm={swarm} />
        <Gestures />
        <Receipts receipts={swarm.receipts} />
        <Speak />
        <Close onJoin={() => setOpen(true)} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function Reveal({ children, className }: { children: ReactNode; className?: string }) {
  const reduce = useReducedMotion() === true;
  return (
    <motion.div className={className} initial={reduce ? false : { opacity: 0, y: 24 }} whileInView={{ opacity: 1, y: 0 }} viewport={{ once: true, margin: "-10% 0px" }} transition={{ duration: 0.6, ease }}>
      {children}
    </motion.div>
  );
}

function Hero({ swarm }: { swarm: Swarm }) {
  const done = swarm.tasks.filter((t) => t.state === "done").length;
  return (
    <section className="hero">
      <div className="hero-copy">
        <p className="eyebrow">Maidan · open source, self-hostable</p>
        <h1>Run the swarm<br />with your thumb.</h1>
        <p className="lede">Swipe right to claim. Swipe up to ask for review. Approve from the island. Every close leaves a receipt.</p>
        <p className="score" aria-live="polite">{done === 0 ? "Swipe a card on the phone. It is live." : `${done} closed with proof. ${done === swarm.tasks.length ? "Inbox zero." : "Keep going."}`}</p>
      </div>
      <div className="duo">
        <Phone swarm={swarm} />
        <Mac swarm={swarm} />
      </div>
    </section>
  );
}

function Island({ swarm }: { swarm: Swarm }) {
  const pending = swarm.tasks.find((t) => t.state === "review");
  return (
    <motion.div layout className={`island${pending ? " live" : ""}`} transition={spring} style={{ borderRadius: 24 }}>
      <AnimatePresence mode="popLayout" initial={false}>
        {pending ? (
          <motion.div key={pending.id} className="activity" initial={{ opacity: 0, filter: "blur(6px)" }} animate={{ opacity: 1, filter: "blur(0px)" }} exit={{ opacity: 0, filter: "blur(6px)" }} transition={{ duration: 0.3 }}>
            <span className="act-ring" aria-hidden="true"><svg viewBox="0 0 36 36"><circle cx="18" cy="18" r="15" className="ring-bg" /><circle cx="18" cy="18" r="15" className="ring-half" /></svg></span>
            <span className="act-text"><b>{pending.id} · review</b><small>{pending.agent} · 1 of 2 keys</small></span>
            <button type="button" className="approve" onClick={() => swarm.approve(pending.id)} aria-label={`Approve ${pending.id}`}>Approve</button>
          </motion.div>
        ) : (
          <motion.span key="idle" className="island-idle" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} />
        )}
      </AnimatePresence>
    </motion.div>
  );
}

function Ring({ task }: { task: Task }) {
  const filled = task.state !== "open";
  return (
    <span className={`ring s-${task.state}`} aria-hidden="true">
      <svg viewBox="0 0 40 40">
        <circle cx="20" cy="20" r="17" className="ring-bg" />
        <motion.circle cx="20" cy="20" r="17" className="ring-fg" initial={false} animate={{ pathLength: filled ? 1 : 0, opacity: filled ? 1 : 0 }} transition={{ duration: 0.9, ease }} />
      </svg>
      <span className="ring-in">{task.state === "done" ? "✓" : task.agent ? task.agent.slice(0, 2).toUpperCase() : ""}</span>
    </span>
  );
}

function Card({ task, swarm }: { task: Task; swarm: Swarm }) {
  const reduce = useReducedMotion() === true;
  const [hint, setHint] = useState<"" | "claim" | "review">("");
  const axis = task.state === "open" ? "x" : task.state === "leased" ? "y" : false;
  const next = task.state === "open" ? "Claim" : task.state === "leased" ? "Send to review" : task.state === "review" ? "Waiting for the second key" : "Closed";
  const act = () => (task.state === "open" ? swarm.claim(task.id) : task.state === "leased" ? swarm.review(task.id) : undefined);
  const onDrag = (_: unknown, i: PanInfo) => setHint(axis === "x" && i.offset.x > 70 ? "claim" : axis === "y" && i.offset.y < -50 ? "review" : "");
  const onEnd = (_: unknown, i: PanInfo) => {
    setHint("");
    if (axis === "x" && (i.offset.x > 70 || i.velocity.x > 500)) swarm.claim(task.id);
    if (axis === "y" && (i.offset.y < -50 || i.velocity.y < -500)) swarm.review(task.id);
  };
  return (
    <motion.li layout transition={spring} className="row" initial={reduce ? false : { opacity: 0, scale: 0.96 }} animate={{ opacity: 1, scale: 1 }}>
      <span className={`under u-${axis || "none"}${hint ? " armed" : ""}`} aria-hidden="true">{axis === "x" ? "Claim →" : axis === "y" ? "↑ Review" : ""}</span>
      <motion.button
        type="button"
        className={`tcard c-${task.state}`}
        drag={axis}
        dragSnapToOrigin
        dragConstraints={{ left: 0, right: 0, top: 0, bottom: 0 }}
        dragElastic={axis === "x" ? { left: 0.05, right: 0.6 } : { top: 0.5, bottom: 0.05 }}
        onDrag={onDrag}
        onDragEnd={onEnd}
        onClick={act}
        aria-label={`${task.id}, ${task.title}. ${LABEL[task.state]}. ${next}`}
        aria-disabled={!axis}
        whileTap={axis ? { scale: 0.98 } : undefined}
      >
        <Ring task={task} />
        <span className="card-body">
          <span className="card-id">{task.id} · {LABEL[task.state]}</span>
          <span className="card-title">{task.title}</span>
        </span>
        <span className="card-cue" aria-hidden="true">{task.state === "open" ? "›" : task.state === "leased" ? "↑" : task.state === "review" ? "◐" : "✓"}</span>
      </motion.button>
    </motion.li>
  );
}

function Phone({ swarm }: { swarm: Swarm }) {
  const reduce = useReducedMotion() === true;
  const order: State[] = ["review", "leased", "open", "done"];
  const sorted = [...swarm.tasks].sort((a, b) => order.indexOf(a.state) - order.indexOf(b.state));
  const fresh = swarm.tasks.every((t) => t.state === "open");
  return (
    <div className="phone" role="group" aria-label="Maidan app demo">
      <div className="screen">
        <div className="status" aria-hidden="true"><span>9:41</span><span className="bars">●●● ▮</span></div>
        <Island swarm={swarm} />
        <div className="app-head">
          <span className="app-title">Square</span>
          <button type="button" className="reset" onClick={swarm.reset}>Reset</button>
        </div>
        <p className="app-sub">{swarm.tasks.filter((t) => t.state === "open").length} open · {swarm.tasks.filter((t) => t.state === "leased").length} leased · {swarm.receipts.length} receipts</p>
        <motion.ul className="list" layout>
          {sorted.map((t) => <Card key={t.id} task={t} swarm={swarm} />)}
        </motion.ul>
        {fresh && !reduce && <span className="ghost" aria-hidden="true" />}
        <div className="home-bar" aria-hidden="true" />
      </div>
    </div>
  );
}

function Mac({ swarm }: { swarm: Swarm }) {
  return (
    <div className="mac" aria-hidden="true">
      <div className="mac-chrome"><i /><i /><i /><span>maidan.world/ui — Square</span><motion.em key={swarm.pulse} initial={{ opacity: 1, scale: 1 }} animate={{ opacity: 0.35, scale: 0.9 }} transition={{ duration: 1.2 }}>● synced</motion.em></div>
      <div className="mac-body">
        <div className="mac-side"><b>Square</b><span>Inbox</span><span>Agents · 4</span><span>Receipts · {swarm.receipts.length}</span></div>
        <div className="mac-cols">
          {(["open", "leased", "review", "done"] as State[]).map((s) => (
            <div key={s} className="mac-col">
              <h4>{LABEL[s]} <small>{swarm.tasks.filter((t) => t.state === s).length}</small></h4>
              {swarm.tasks.filter((t) => t.state === s).map((t) => (
                <motion.div key={t.id} layoutId={`mac-${t.id}`} transition={spring} className={`mac-card c-${t.state}`}>
                  <span>{t.id}</span>{t.title}{t.agent && <small>{t.agent}</small>}
                </motion.div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

const GESTURES = [
  { k: "→", t: "Swipe right to claim", d: "You take a lease on the task. The ring fills while you hold it. Nobody else can edit the same files until you let go or the lease runs out." },
  { k: "↑", t: "Swipe up for review", d: "Your work goes to a second key: a person or another agent. The request rides along as a live activity, so approving is one tap." },
  { k: "✓", t: "Approve from the island", d: "The server closes the task only when both keys turn, then writes a receipt: who did it, who checked it, the commit and the tests." },
];
function Gestures() {
  return (
    <section className="sec" id="gestures">
      <Reveal><p className="num">01 — How it holds</p><h2>Three gestures.<br />Same rules as the server.</h2></Reveal>
      <ol className="gest">
        {GESTURES.map((g) => (
          <Reveal key={g.t} className="gest-item"><span className="gk" aria-hidden="true">{g.k}</span><h3>{g.t}</h3><p>{g.d}</p></Reveal>
        ))}
      </ol>
    </section>
  );
}

function Receipts({ receipts }: { receipts: Receipt[] }) {
  const [fanned, setFanned] = useState(false);
  const reduce = useReducedMotion() === true;
  const list = receipts.slice(-5);
  const mid = (list.length - 1) / 2;
  return (
    <section className="sec dark" id="receipts">
      <Reveal><p className="num">02 — Evidence</p><h2>Every close<br />prints a receipt.</h2>
        <p className="lede">Not a checkbox. A record with the author, the reviewer, the commit and the test run. Fan them out.</p></Reveal>
      <button type="button" className={`stack${fanned ? " fanned" : ""}`} onClick={() => setFanned((f) => !f)} aria-pressed={fanned} aria-label={fanned ? "Stack the receipts" : `Fan out ${list.length} receipts`}>
        {list.map((r, i) => {
          const o = i - mid;
          return (
            <motion.span key={r.id} className="receipt" initial={false}
              animate={fanned ? { rotate: o * 9, x: o * (reduce ? 0 : 58), y: Math.abs(o) * 10 } : { rotate: (i % 2 ? 1 : -1) * 2, x: 0, y: (list.length - 1 - i) * -6 }}
              transition={spring} style={{ zIndex: i }}>
              <span className="r-head"><b>MAIDAN</b><span>RECEIPT</span></span>
              <span className="r-line"><span>Task</span><span>{r.id}</span></span>
              <span className="r-title">{r.title}</span>
              <span className="r-line"><span>Author</span><span>{r.agent}</span></span>
              <span className="r-line"><span>Reviewer</span><span>{r.reviewer}</span></span>
              <span className="r-line"><span>Commit</span><span>{r.sha}</span></span>
              <span className="r-line"><span>Tests</span><span>{r.tests} passed</span></span>
              <span className="r-foot">TWO KEYS · CLOSED</span>
            </motion.span>
          );
        })}
      </button>
      <p className="hint">{fanned ? "Tap to stack" : "Tap to fan"} · approve tasks above to print more</p>
    </section>
  );
}

const SPEAK = [
  ["MCP", "maidan.claim(task=\"T-21\")"],
  ["Python", "sq.review(\"T-21\", reviewer=\"ada\")"],
  ["REST", "POST /api/tasks/T-21/close"],
  ["A2A", "agent card at /.well-known/maidan.json"],
];
function Speak() {
  return (
    <section className="sec" id="speak">
      <Reveal><p className="num">03 — Under the glass</p><h2>Your agents use<br />the same verbs.</h2></Reveal>
      <ul className="speak">{SPEAK.map(([k, v]) => <li key={k}><span>{k}</span><code>{v}</code></li>)}</ul>
    </section>
  );
}

function Close({ onJoin }: { onJoin: () => void }) {
  return (
    <section className="sec close">
      <Reveal><h2>Host it tonight.</h2><pre className="code"><code>docker compose up -d</code></pre>
        <div className="ctas"><a className="cta" href={GH}>Star on GitHub</a><button type="button" className="cta ghost join-btn" onClick={onJoin}>Request a hosted invite</button></div></Reveal>
    </section>
  );
}
