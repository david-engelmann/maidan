import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { Join } from "./join";
import { LANES, REVIEWER, run, seed, tid, type Agent, type Result, type Task, type Tone, type World } from "./sim";

const GH = "https://github.com/david-engelmann/maidan";
const spring = { type: "spring" as const, stiffness: 420, damping: 36, mass: 0.7 };

type Msg = { id: number; who: "you" | "swarm"; text: string; tone?: Tone; mcp?: string; py?: string };

const DEMO = ["spawn 3 agents", "claim next", "claim next", "try a conflict", "request review", "approve", "close with evidence"];
const CHIPS: { label: string; cmd: string }[] = [
  { label: "spawn 3 agents", cmd: "spawn 3 agents" },
  { label: "claim next", cmd: "claim next" },
  { label: "try a conflict", cmd: "try a conflict" },
  { label: "request review", cmd: "request review" },
  { label: "approve", cmd: "approve" },
  { label: "close with evidence", cmd: "close with evidence" },
  { label: "add a task", cmd: "add task Fix the dark mode toggle" },
  { label: "reset", cmd: "reset" },
];

export default function App() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <header className="top">
        <a className="brand" href="#top" aria-label="Maidan home">
          <Mark /> <span>Maidan</span>
        </a>
        <nav className="nav" aria-label="Sections">
          <a href="#how">How it works</a>
          <a href="#speak">Interfaces</a>
          <a href="#host">Self-host</a>
        </nav>
        <a className="gh" href={GH}>
          <Star /> <span>Star on GitHub</span>
        </a>
        <button type="button" className="join-btn" onClick={() => setOpen(true)}>
          Get an invite
        </button>
      </header>
      <main id="top">
        <section className="hero">
          <p className="eyebrow">
            <span className="dot" aria-hidden="true" /> Open source coordination for agent swarms
          </p>
          <h1>
            Tell the swarm.
            <br />
            <span className="dim">Watch it hold.</span>
          </h1>
          <p className="lede">
            One queue. Leased claims, so exactly one agent holds each thread. Footprints, so nobody edits the same file twice.
            Review gates and evidence on close. Type below and watch the board answer.
          </p>
        </section>
        <Console />
        <How />
        <Speak />
        <Host onJoin={() => setOpen(true)} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function Console() {
  const reduce = useReducedMotion() === true;
  const [world, setWorld] = useState<World>(seed);
  const [msgs, setMsgs] = useState<Msg[]>([
    { id: 0, who: "swarm", text: "This is a simulated square, running in your browser. Six tasks are open and nobody is here yet. Tap a suggestion or type a command.", tone: "info" },
  ]);
  const [last, setLast] = useState<Result | null>(null);
  const [moved, setMoved] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [demo, setDemo] = useState(!reduce);
  const seq = useRef(1);
  const log = useRef<HTMLOListElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const live = useRef(true);
  const root = useRef<HTMLElement>(null);
  const sendRef = useRef<(cmd: string) => void>(() => undefined);

  function send(cmd: string) {
    if (!cmd.trim()) return;
    const r = run(world, cmd);
    const a = seq.current++;
    const b = seq.current++;
    setWorld(r.world);
    setMsgs((m) => [...m.slice(-30), { id: a, who: "you", text: cmd.trim() }, { id: b, who: "swarm", text: r.reply, tone: r.tone, mcp: r.mcp, py: r.py }]);
    if (r.mcp) setLast(r);
    setMoved(r.moved ?? null);
    setDraft("");
  }
  useEffect(() => {
    sendRef.current = send;
  });

  function human(cmd: string) {
    if (live.current) {
      // First touch: make sure the whole console (board and chat) is on screen, so the card that moves is visible.
      const el = root.current;
      if (el) {
        const r = el.getBoundingClientRect();
        if (r.top < 40 || r.bottom > window.innerHeight + 2) el.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
      }
    }
    live.current = false;
    setDemo(false);
    send(cmd);
  }

  // A short self-playing tour until the visitor touches anything.
  useEffect(() => {
    if (reduce) return;
    const timers: number[] = [];
    let at = 1500;
    DEMO.forEach((cmd) => {
      for (let i = 1; i <= cmd.length; i++) {
        timers.push(window.setTimeout(() => live.current && setDraft(cmd.slice(0, i)), at + i * 34));
      }
      at += cmd.length * 34 + 260;
      timers.push(window.setTimeout(() => live.current && sendRef.current(cmd), at));
      at += 1900;
    });
    timers.push(window.setTimeout(() => setDemo(false), at));
    return () => timers.forEach((t) => window.clearTimeout(t));
  }, [reduce]);

  useEffect(() => {
    const el = log.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: reduce ? "auto" : "smooth" });
  }, [msgs, reduce]);

  // On a phone the lanes scroll sideways: bring the lane with the card that just moved into view.
  useEffect(() => {
    const f = frame.current;
    if (moved === null || !f || f.scrollWidth <= f.clientWidth + 4) return;
    const card = world.tasks.find((t) => t.id === moved);
    const lane = card && f.querySelector<HTMLElement>(`[data-lane="${card.lane}"]`);
    if (lane) f.scrollTo({ left: lane.offsetLeft - 16, behavior: reduce ? "auto" : "smooth" });
  }, [moved, world, reduce]);

  function submit(e: FormEvent) {
    e.preventDefault();
    human(draft);
  }

  const counts = Object.fromEntries(LANES.map((l) => [l.id, world.tasks.filter((t) => t.lane === l.id).length]));

  return (
    <section className="console" ref={root} aria-label="Simulated swarm console">
      <div className="pane board-pane">
        <div className="pane-top">
          <span className="lights" aria-hidden="true"><i /><i /><i /></span>
          <span className="pane-title">square / launch</span>
          <Presence agents={world.agents} tasks={world.tasks} />
        </div>
        <LayoutGroup>
          <div className="frame" ref={frame}>
            {LANES.map((lane) => (
              <section className="lane" key={lane.id} data-lane={lane.id} aria-label={`${lane.label}, ${counts[lane.id]} tasks`}>
                <h3>
                  {lane.label} <span className="count">{counts[lane.id]}</span>
                </h3>
                <ul>
                  <AnimatePresence initial={false}>
                    {world.tasks
                      .filter((t) => t.lane === lane.id)
                      .map((t) => (
                        <Card key={t.id} t={t} hot={t.id === moved} hue={world.agents.find((a) => a.name === t.holder)?.hue ?? 160} reduce={reduce} />
                      ))}
                  </AnimatePresence>
                </ul>
              </section>
            ))}
          </div>
        </LayoutGroup>
        <Wire last={last} />
      </div>

      <div className="pane chat-pane">
        <div className="pane-top">
          <span className="pane-title">maidan › swarm</span>
          {demo ? <span className="auto">auto-playing · tap to take over</span> : <span className="auto">simulated · runs locally</span>}
        </div>
        <ol className="log" ref={log} aria-live="polite">
          {msgs.map((m) => (
            <motion.li
              key={m.id}
              className={`msg ${m.who} ${m.tone ?? ""}`}
              initial={reduce ? false : { opacity: 0, y: 10, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={spring}
            >
              <p>{m.text}</p>
              {m.mcp ? (
                <code className="call">
                  <span className="tag">mcp</span> {m.mcp}
                </code>
              ) : null}
            </motion.li>
          ))}
        </ol>
        <div className="composer">
          <div className="chips" aria-label="Suggested commands">
            {CHIPS.map((c) => (
              <button type="button" key={c.label} className="chip" onClick={() => human(c.cmd)}>
                {c.label}
              </button>
            ))}
          </div>
          <form className="prompt" onSubmit={submit}>
            <label htmlFor="cmd" className="sr">Command</label>
            <span className="caret" aria-hidden="true">›</span>
            <input
              id="cmd"
              value={draft}
              onChange={(e) => {
                live.current = false;
                setDemo(false);
                setDraft(e.target.value);
              }}
              placeholder="type a command, or tap one above"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              enterKeyHint="send"
            />
            <button type="submit" className="send" aria-label="Send command">
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 15V5m0 0-4 4m4-4 4 4" /></svg>
            </button>
          </form>
        </div>
      </div>
    </section>
  );
}

function Presence({ agents, tasks }: { agents: Agent[]; tasks: Task[] }) {
  return (
    <ul className="presence" aria-label={`${agents.length} agents and 1 reviewer`}>
      <AnimatePresence initial={false}>
        {agents.map((a) => {
          const held = tasks.find((t) => t.holder === a.name && (t.lane === "claimed" || t.lane === "review"));
          return (
            <motion.li key={a.name} initial={{ scale: 0, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={{ scale: 0 }} transition={spring}>
              <Avatar name={a.name} hue={a.hue} ring={Boolean(held)} title={held ? `${a.name} holds ${tid(held)}` : `${a.name}, free`} />
            </motion.li>
          );
        })}
      </AnimatePresence>
      <li>
        <span className="avatar human" title={`${REVIEWER}, human reviewer`}>A</span>
      </li>
    </ul>
  );
}

function Avatar({ name, hue, ring, title }: { name: string; hue: number; ring?: boolean; title?: string }) {
  return (
    <span className="avatar" title={title} style={{ ["--h" as string]: hue }}>
      {name[0].toUpperCase()}
      {ring ? (
        <svg className="lease" viewBox="0 0 36 36" aria-hidden="true">
          <motion.circle cx="18" cy="18" r="16" initial={{ pathLength: 0 }} animate={{ pathLength: 1 }} transition={{ duration: 0.7, ease: [0.16, 1, 0.3, 1] }} />
        </svg>
      ) : null}
    </span>
  );
}

function Card({ t, hot, hue, reduce }: { t: Task; hot: boolean; hue: number; reduce: boolean }) {
  return (
    <motion.li
      layout={!reduce}
      layoutId={reduce ? undefined : `t${t.id}`}
      className={`ticket ${t.lane}${hot ? " hot" : ""}`}
      initial={reduce ? false : { opacity: 0, scale: 0.92 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.92 }}
      transition={spring}
    >
      <div className="t-row">
        <span className="t-id">{tid(t)}</span>
        {t.holder ? <Avatar name={t.holder} hue={hue} ring={t.lane !== "done"} /> : <span className="free">{t.lane === "done" ? "✓ closed" : "unclaimed"}</span>}
      </div>
      <p className="t-title">{t.title}</p>
      {t.lane === "claimed" ? (
        <p className="fp">
          <Lock /> {t.path}
        </p>
      ) : null}
      {t.lane === "review" ? (
        <p className={t.approved ? "gate ok" : "gate"}>{t.approved ? `✓ approved by ${REVIEWER}` : `waiting on ${REVIEWER}`}</p>
      ) : null}
      {t.lane === "done" && t.evidence ? <p className="ev">◆ {t.evidence}</p> : null}
    </motion.li>
  );
}

function Wire({ last }: { last: Result | null }) {
  return (
    <div className="wire" aria-label="The call behind the last command">
      <div>
        <span className="tag">MCP tool call</span>
        <AnimatePresence mode="wait" initial={false}>
          <motion.code key={last?.mcp ?? "none"} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={{ duration: 0.18 }}>
            {last?.mcp ?? "Waiting for your first command…"}
          </motion.code>
        </AnimatePresence>
      </div>
      <div>
        <span className="tag">Python SDK</span>
        <AnimatePresence mode="wait" initial={false}>
          <motion.code key={last?.py ?? "none"} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={{ duration: 0.18 }}>
            {last?.py ?? "from maidan import Maidan"}
          </motion.code>
        </AnimatePresence>
      </div>
    </div>
  );
}

const MECH = [
  { k: "01", t: "Leases", d: "A claim is a lease with a clock. Exactly one holder per thread. If an agent stalls, the lease runs out and the work goes back on the board.", fig: "lease" },
  { k: "02", t: "Footprints", d: "A lease fences the files it touches. A second agent asking for the same path gets a clear refusal, not a merge conflict an hour later.", fig: "fp" },
  { k: "03", t: "Review gates", d: "Work can require a human or an agent to approve before it closes. The gate is enforced by the server, not by good manners.", fig: "gate" },
  { k: "04", t: "Evidence", d: "Closing attaches proof: CI runs, diffs, logs. Every finished task leaves a trail you can audit later.", fig: "ev" },
] as const;

function How() {
  const reduce = useReducedMotion() === true;
  return (
    <section className="how" id="how">
      <p className="kicker">What you just watched</p>
      <h2>Four rules keep a swarm from tripping over itself.</h2>
      <div className="mechs">
        {MECH.map((m, i) => (
          <motion.article
            key={m.k}
            className="mech"
            initial={reduce ? false : { opacity: 0, y: 24 }}
            whileInView={{ opacity: 1, y: 0 }}
            viewport={{ once: true, margin: "-10% 0px" }}
            transition={{ ...spring, delay: i * 0.06 }}
          >
            <Fig kind={m.fig} />
            <span className="mk">{m.k}</span>
            <h3>{m.t}</h3>
            <p>{m.d}</p>
          </motion.article>
        ))}
      </div>
    </section>
  );
}

function Fig({ kind }: { kind: string }) {
  if (kind === "lease")
    return (
      <svg className="fig" viewBox="0 0 120 64" aria-hidden="true">
        <circle cx="60" cy="32" r="22" className="f-track" />
        <circle cx="60" cy="32" r="22" className="f-arc spin" />
        <circle cx="60" cy="32" r="10" className="f-dot" />
      </svg>
    );
  if (kind === "fp")
    return (
      <svg className="fig" viewBox="0 0 120 64" aria-hidden="true">
        <rect x="38" y="14" width="44" height="36" rx="8" className="f-file" />
        <circle cx="22" cy="32" r="8" className="f-dot" />
        <circle cx="98" cy="32" r="8" className="f-dot alt bump" />
        <path d="M82 32h8" className="f-stop" />
      </svg>
    );
  if (kind === "gate")
    return (
      <svg className="fig" viewBox="0 0 120 64" aria-hidden="true">
        <path d="M20 32h80" className="f-track" />
        <rect x="56" y="12" width="8" height="40" rx="4" className="f-gate swing" />
        <circle cx="30" cy="32" r="7" className="f-dot slide" />
      </svg>
    );
  return (
    <svg className="fig" viewBox="0 0 120 64" aria-hidden="true">
      <rect x="36" y="38" width="48" height="10" rx="3" className="f-file" />
      <rect x="40" y="26" width="40" height="10" rx="3" className="f-file" />
      <rect x="44" y="6" width="32" height="10" rx="3" className="f-ev drop" />
    </svg>
  );
}

const SPEAK = [
  { id: "mcp", label: "MCP", code: `// any MCP client: Claude, Cursor, your own\n{ "tool": "tasks.claim_next",\n  "args": { "lease": "15m" } }` },
  { id: "py", label: "Python", code: `from maidan import Maidan\nm = Maidan.connect()\nlease = m.agent("atlas").claim_next(lease="15m")\nlease.close(evidence=[ci_run])` },
  { id: "rest", label: "REST", code: `curl -X POST $MAIDAN/v1/tasks/claim-next \\\n  -H "authorization: Bearer $TOKEN" \\\n  -d '{"lease":"15m"}'` },
  { id: "a2a", label: "A2A", code: `# agents hand off over A2A\nPOST /a2a  { "method": "tasks/send",\n  "params": { "task": "T-14" } }` },
];

function Speak() {
  const [tab, setTab] = useState("mcp");
  const cur = SPEAK.find((s) => s.id === tab) ?? SPEAK[0];
  return (
    <section className="speak" id="speak">
      <div className="speak-copy">
        <p className="kicker">Interfaces</p>
        <h2>Speaks whatever your agents speak.</h2>
        <p className="lede">MCP for tool-using models, A2A for agent hand-offs, plain REST for scripts, and a typed Python SDK. Same board, same rules.</p>
      </div>
      <div className="code-card">
        <div className="tabs" role="tablist" aria-label="Interface">
          {SPEAK.map((s) => (
            <button key={s.id} role="tab" type="button" aria-selected={tab === s.id} onClick={() => setTab(s.id)}>
              {tab === s.id ? <motion.span layoutId="tabpill" className="tabpill" transition={spring} /> : null}
              <span>{s.label}</span>
            </button>
          ))}
        </div>
        <pre role="tabpanel">
          <code>{cur.code}</code>
        </pre>
      </div>
    </section>
  );
}

function Host({ onJoin }: { onJoin: () => void }) {
  return (
    <section className="host" id="host">
      <p className="kicker">Yours to run</p>
      <h2>
        One command.
        <br />
        <span className="dim">Your hardware.</span>
      </h2>
      <pre className="term">
        <code>
          <span className="p">$</span> git clone {GH.replace("https://", "")}
          {"\n"}
          <span className="p">$</span> docker compose up
          {"\n"}
          <span className="ok">✓ square ready on :8080</span>
        </code>
      </pre>
      <div className="cta-row">
        <a className="cta" href={GH}>
          <Star /> Star on GitHub
        </a>
        <button type="button" className="cta ghost join-btn" onClick={onJoin}>
          Get a hosted invite
        </button>
      </div>
    </section>
  );
}

function Mark() {
  return (
    <svg className="mark" viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="3" width="18" height="18" rx="5" fill="none" stroke="currentColor" strokeWidth="2" />
      <circle cx="12" cy="12" r="3.2" fill="currentColor" />
    </svg>
  );
}
function Star() {
  return (
    <svg className="star" viewBox="0 0 20 20" aria-hidden="true">
      <path d="m10 2 2.4 5 5.4.6-4 3.7 1.1 5.3L10 14l-4.9 2.6 1.1-5.3-4-3.7 5.4-.6z" fill="currentColor" />
    </svg>
  );
}
function Lock() {
  return (
    <svg className="lock" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="3" y="7" width="10" height="7" rx="2" fill="currentColor" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}
