import { AnimatePresence, motion, useAnimate, useInView, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState, type ReactNode, type KeyboardEvent, type PointerEvent as RPointerEvent } from "react";
import { Circuit, EMPTY, ROLES, type Wells } from "./circuit";
import { FILM, onCue, playFilm } from "./film";
import { Join } from "./join";

const GH = "https://github.com/david-engelmann/maidan";
const ease = [0.16, 1, 0.3, 1] as const;
const spring = { type: "spring" as const, stiffness: 380, damping: 32, mass: 0.8 };
const shake = { x: [0, -10, 9, -6, 4, 0], transition: { duration: 0.45 } };

export default function App() {
  const [open, setOpen] = useState(false);
  const [wells, setWells] = useState<Wells>(EMPTY);
  const [scrolled, setScrolled] = useState(false);
  const [dark, setDark] = useState(false);
  useEffect(() => (FILM ? playFilm() : undefined), []);
  useEffect(() => {
    const on = () => {
      setScrolled(window.scrollY > 8);
      // The bar sits over the dark Scale section: switch to the dark glass.
      const r = document.getElementById("hundred")?.getBoundingClientRect();
      setDark(!!r && r.top <= 32 && r.bottom >= 32);
    };
    on();
    window.addEventListener("scroll", on, { passive: true });
    window.addEventListener("resize", on);
    return () => { window.removeEventListener("scroll", on); window.removeEventListener("resize", on); };
  }, []);
  return (
    <>
      <header className={`top${FILM ? " film" : ""}${scrolled ? " scrolled" : ""}${dark ? " dark" : ""}`}>
        <a className="brand" href="#top">Maidan</a>
        <nav className="nav" aria-label="Sections">
          <a href="#hundred">Scale</a>
          <a href="#lease">Lease</a>
          <a href="#review">Review</a>
          <a href="#evidence">Evidence</a>
        </nav>
        <a className="gh" href={GH}>★ Star</a>
        <button type="button" className="join-btn" onClick={() => setOpen(true)}>Invite</button>
      </header>
      <main id="top">
        <section className="hero">
          <div className="hero-copy">
            <p className="eyebrow">Coordination for agent swarms<span className="eb-more"> · open source</span></p>
            <h1>Agents claim.<br />You approve.</h1>
            <p className="lede">Run several coding agents and they collide: two grab one task, or one redoes another’s work. Maidan gives each task one holder and a second member’s review before it closes. Place the four roles to see it.</p>
          </div>
          <Circuit wells={wells} setWells={setWells} />
        </section>
        <Hundred />
        <Lease />
        <Review />
        <Evidence />
        <Run wells={wells} onJoin={() => setOpen(true)} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function Reveal({ children, className }: { children: ReactNode; className?: string }) {
  const reduce = useReducedMotion() === true;
  return (
    <motion.div className={className} initial={reduce ? false : { opacity: 0, y: 28 }} whileInView={{ opacity: 1, y: 0 }} viewport={{ once: true, margin: "-12% 0px" }} transition={{ duration: 0.9, ease }}>
      {children}
    </motion.div>
  );
}

/* Now make it a hundred */
function Hundred() {
  const reduce = useReducedMotion() === true;
  const ref = useRef<HTMLElement>(null);
  const seen = useInView(ref, { once: true, amount: 0.45 });
  const live = useInView(ref);
  const [closed, setClosed] = useState(0);
  useEffect(() => {
    if (!seen || !live) return;
    const t = window.setInterval(() => setClosed((c) => c + 7), 160);
    return () => window.clearInterval(t);
  }, [seen, live]);
  return (
    <section className={live ? "hundred" : "hundred paused"} id="hundred" ref={ref}>
      <div className="hundred-copy">
        <p className="num">01 · Scale</p>
        <h2>Now make it<br />a hundred.</h2>
      </div>
      <div className="wall-frame">
        <motion.div
          className="wall"
          aria-hidden="true"
          initial={reduce ? false : { scale: 7.2 }}
          animate={seen || reduce ? { scale: 1 } : { scale: 7.2 }}
          transition={{ duration: 3.4, ease: [0.65, 0, 0.15, 1], delay: 0.2 }}
        >
          {Array.from({ length: 100 }, (_, i) => <i key={i} className={i === 44 ? "mine" : undefined} />)}
        </motion.div>
      </div>
      <dl className="stats" aria-live="off">
        <div><dt>tiles</dt><dd>100</dd></div>
        <div><dt>tasks closed</dt><dd>{closed.toLocaleString("en-US")}</dd></div>
        <div className="zero"><dt>double claims</dt><dd>0</dd></div>
      </dl>
      <p className="lede center">More agents means more of them reaching for the same work. They all draw from one queue, and the server gives each task to one claimant at a time, so two agents never hold the same task. This wall is an illustration, not a benchmark.</p>
    </section>
  );
}

/* Lease dial */
function Lease() {
  const [min, setMin] = useState(25);
  const [running, setRunning] = useState(false);
  const knob = useRef<SVGSVGElement>(null);
  const dragging = useRef(false);
  const angle = (min / 60) * 300 - 150;
  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => setMin((m) => { if (m <= 1) { setRunning(false); return 0; } return m - 1; }), 70);
    return () => window.clearInterval(t);
  }, [running]);
  useEffect(() => onCue((n) => { if (n === "lease:expire") setRunning(true); }), []);

  function fromPointer(e: RPointerEvent<SVGSVGElement>) {
    const el = knob.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const a = (Math.atan2(e.clientX - (r.left + r.width / 2), -(e.clientY - (r.top + r.height / 2))) * 180) / Math.PI;
    setRunning(false);
    setMin(Math.round(((Math.max(-150, Math.min(150, a)) + 150) / 300) * 12) * 5);
  }
  function onKey(e: KeyboardEvent) {
    const step = (v: number) => { e.preventDefault(); setRunning(false); setMin(v); };
    if (e.key === "ArrowUp" || e.key === "ArrowRight") step(Math.min(60, min + 5));
    if (e.key === "ArrowDown" || e.key === "ArrowLeft") step(Math.max(0, min - 5));
    if (e.key === "Home") step(0);
    if (e.key === "End") step(60);
  }
  const expired = min === 0;
  return (
    <section className="scene" id="lease">
      <Reveal className="scene-copy">
        <p className="num">02 · Lease</p>
        <h2>One holder.<br />A clock on it.</h2>
        <p className="lede">A claim says “this task is mine.” A lease is the clock on that claim. While the Coder works it renews the lease, and nobody else can take the task. If the Coder crashes or goes quiet, the lease runs out and the task goes back to the queue for anyone to claim. Nothing stays stuck on a dead agent.</p>
      </Reveal>
      <div className="object">
        <svg ref={knob} className="dial" viewBox="0 0 240 240" role="slider" tabIndex={0} aria-label="Lease length" aria-valuemin={0} aria-valuemax={60} aria-valuenow={min}
          aria-valuetext={expired ? "Lease expired, task returned to the queue" : `${min} minutes left on the Coder's lease`}
          onKeyDown={onKey}
          onPointerDown={(e) => { dragging.current = true; e.currentTarget.setPointerCapture(e.pointerId); fromPointer(e); }}
          onPointerMove={(e) => { if (dragging.current) fromPointer(e); }}
          onPointerUp={() => { dragging.current = false; }}
        >
          <defs>
            <radialGradient id="cer" cx="40%" cy="30%" r="80%"><stop offset="0" stopColor="#fffdf8" /><stop offset="0.7" stopColor="#ebe7de" /><stop offset="1" stopColor="#d6d1c5" /></radialGradient>
          </defs>
          {Array.from({ length: 61 }, (_, i) => {
            const a = ((i / 60) * 300 - 150) * (Math.PI / 180);
            const r1 = i % 5 === 0 ? 99 : 104;
            return <line key={i} x1={120 + Math.sin(a) * r1} y1={120 - Math.cos(a) * r1} x2={120 + Math.sin(a) * 112} y2={120 - Math.cos(a) * 112} className={i <= min && min > 0 ? "tick on" : "tick"} />;
          })}
          <circle cx="120" cy="126" r="82" className="knob-shadow" />
          <circle cx="120" cy="120" r="80" fill="url(#cer)" className="knob" />
          <circle cx="120" cy="120" r="62" className="knob-face" />
          <g transform="translate(120 120)">
            <motion.g animate={{ rotate: angle }} transition={spring}>
              <circle r="80" fill="transparent" />
              <rect x="-3" y="-76" width="6" height="22" rx="3" className="notch" />
            </motion.g>
          </g>
          <text x="120" y="130" textAnchor="middle" className="dial-read">{expired ? "00′" : `${min}′`}</text>
        </svg>
        <div className="rail" aria-hidden="true">
          <span className="rail-stop"><span className="rail-well" />queue</span>
          <span className="rail-track"><motion.span className="token" initial={false} animate={{ left: expired ? "0%" : "100%" }} transition={{ type: "spring", stiffness: 120, damping: 14 }} /></span>
          <span className="rail-stop"><span className="rail-well"><span className="puck coder mini"><span className="puck-label">C</span></span></span>coder</span>
        </div>
        <p className="sr" aria-live="polite">{running ? "The lease is running down." : expired ? "Lease expired. T-14 rolled back to the queue. Anyone can claim it." : ""}</p>
        <p className="readout" aria-hidden={running || undefined}>
          {expired ? <><b>Lease expired.</b> T-14 rolled back to the queue. Anyone can claim it.</> : <><b>Coder</b> holds T-14 for {min} more minutes. Nobody else can.</>}
        </p>
        <button type="button" className="key-btn" onClick={() => (expired ? setMin(25) : setRunning(true))}>{expired ? "Claim it again" : "Let the clock run out"}</button>
      </div>
    </section>
  );
}

/* Two-key lock */
function Review() {
  const [keys, setKeys] = useState({ author: false, reviewer: false });
  const [merged, setMerged] = useState(false);
  const [refused, setRefused] = useState(false);
  const [scope, animate] = useAnimate();
  const both = keys.author && keys.reviewer;
  function merge() {
    if (both) { setMerged(true); setRefused(false); return; }
    setRefused(true);
    void animate(scope.current, shake);
  }
  const turn = (k: "author" | "reviewer") => { setMerged(false); setRefused(false); setKeys((s) => ({ ...s, [k]: !s[k] })); };
  const mergeRef = useRef(merge);
  useEffect(() => { mergeRef.current = merge; });
  useEffect(() => onCue((n) => {
    if (n === "lock:author") turn("author");
    if (n === "lock:reviewer") turn("reviewer");
    if (n === "lock:merge") mergeRef.current();
  }), []);
  return (
    <section className="scene alt" id="review">
      <Reveal className="scene-copy">
        <p className="num">03 · Review</p>
        <h2>Two keys<br />to close.</h2>
        <p className="lede">A review is the second key. The worker turns the first by handing its result to review, and someone else turns the other. An agent can’t approve its own work, and an approval gate takes a signed-in person or a grant a person gave. That keeps a person in the loop where it matters: deciding what ships, without watching every step.</p>
      </Reveal>
      <div className="object">
        <svg ref={scope} className="padlock" viewBox="0 0 280 300" role="img" aria-label={merged ? "Padlock open, closed with review" : `Padlock locked, ${Number(keys.author) + Number(keys.reviewer)} of 2 keys turned`}>
          <motion.path d="M80 140 V92 a60 60 0 0 1 120 0 V140" className="shackle" animate={{ y: merged ? -34 : 0 }} transition={{ type: "spring", stiffness: 260, damping: 14 }} />
          <rect x="40" y="130" width="200" height="150" rx="26" className="body" />
          <rect x="40" y="130" width="200" height="150" rx="26" className="body-sheen" />
          <text x="140" y="164" textAnchor="middle" className={refused ? "engrave red" : "engrave"}>{merged ? "CLOSED" : refused ? "REFUSED" : "LOCKED"}</text>
          {(["author", "reviewer"] as const).map((k, i) => {
            const cx = i === 0 ? 96 : 184;
            return (
              <g key={k}>
                <circle cx={cx} cy="212" r="20" className={keys[k] ? "keyhole-plate on" : "keyhole-plate"} />
                <path d={`M${cx - 4} 206 a6 6 0 1 1 8 0 l3 16 h-14 z`} className="keyhole" />
                <g transform={`translate(${cx} 212)`}>
                  <motion.g initial={false} animate={{ opacity: keys[k] ? 1 : 0, rotate: keys[k] ? 90 : 0, y: keys[k] ? 0 : 26 }} transition={{ duration: 0.5, ease }}>
                    <circle r="32" fill="transparent" />
                    <rect x="-3" y="-16" width="6" height="16" rx="2" className="keybit" />
                    <circle cx="0" cy="-23" r="9" className="keybow" />
                    <circle cx="0" cy="-23" r="3" className="keyhole" />
                  </motion.g>
                </g>
                <text x={cx} y="258" textAnchor="middle" className="engrave small">{k === "author" ? "WORKER" : "REVIEWER"}</text>
              </g>
            );
          })}
        </svg>
        <div className="keys">
          <button type="button" className={keys.author ? "key-btn on" : "key-btn"} aria-pressed={keys.author} onClick={() => turn("author")}>Worker key</button>
          <button type="button" className={keys.reviewer ? "key-btn on" : "key-btn"} aria-pressed={keys.reviewer} onClick={() => turn("reviewer")}>Reviewer key</button>
          <button type="button" className="key-btn signal" onClick={merge}>Close T-14</button>
        </div>
        <p className={refused ? "readout bad" : "readout"} aria-live="polite">
          {merged ? <><b>Closed.</b> Approved by a second member, T-14 is done.</> : refused ? <><b>Refused.</b> One key is not a review. Self-approval doesn’t count.</> : both ? "Both keys turned. Try to close." : "Turn a key, then try to close."}
        </p>
      </div>
    </section>
  );
}

/* Sealed envelope */
const SLIPS = [
  { k: "ci", t: "CI run #8812", d: "48 / 48 passed" },
  { k: "diff", t: "Diff", d: "+120 −14 · 3 files" },
  { k: "log", t: "Log", d: "2.1 kB, hashed" },
] as const;
type Slip = (typeof SLIPS)[number]["k"];
function Evidence() {
  const [has, setHas] = useState<Record<Slip, boolean>>({ ci: true, diff: false, log: false });
  const [sealed, setSealed] = useState(false);
  const [refusal, setRefusal] = useState("");
  const [scope, animate] = useAnimate();
  const missing = SLIPS.filter((s) => !has[s.k]);
  function seal() {
    if (sealed) { setSealed(false); return; }
    if (missing.length) {
      setRefusal(`Won’t seal. Missing: ${missing.map((s) => s.t.split(" ")[0]).join(", ")}.`);
      void animate(scope.current, shake);
      return;
    }
    setRefusal("");
    setSealed(true);
  }
  const sealRef = useRef(seal);
  useEffect(() => { sealRef.current = seal; });
  useEffect(() => onCue((n) => {
    if (n === "env:seal") sealRef.current();
    if (n === "env:all") { setRefusal(""); setHas({ ci: true, diff: true, log: true }); }
  }), []);
  const attached = SLIPS.filter((s) => has[s.k]);
  const flapEl = <motion.path key="flap" d="M20 100 L160 196 L300 100 Z" className="flap" initial={sealed ? { scaleY: -1 } : false} animate={{ scaleY: sealed ? 1 : -1 }} transition={{ duration: 0.55, delay: sealed ? 0.35 : 0, ease }} style={{ originX: 0.5, originY: 0 }} />;
  return (
    <section className="scene" id="evidence">
      <Reveal className="scene-copy">
        <p className="num">04 · Evidence</p>
        <h2>Closed means<br />proven.</h2>
        <p className="lede">Handing off to review seals a packet: the result’s hash and every artifact linked to the task. An approval counts only for the packet the reviewer saw. Link something new and the close is refused until someone approves again.</p>
      </Reveal>
      <div className="object">
        <svg ref={scope} className="envelope" viewBox="0 0 320 260" role="img" aria-label={sealed ? "Envelope sealed with three pieces of evidence" : `Open envelope, ${attached.length} of 3 pieces of evidence`}>
          <rect x="20" y="90" width="280" height="160" rx="10" className="env-back" />
          {sealed ? null : flapEl}
          <AnimatePresence>
            {attached.map((s) => {
              const i = SLIPS.findIndex((x) => x.k === s.k);
              return (
                <motion.g key={s.k} initial={{ y: -40, opacity: 0 }} animate={{ y: sealed ? 70 : -i * 6, opacity: 1 }} exit={{ y: -40, opacity: 0 }} transition={{ duration: 0.6, delay: sealed ? i * 0.08 : 0, ease }}>
                  <rect x={44 + i * 8} y={30 + i * 14} width={232 - i * 16} height="96" rx="6" className="slip" />
                  <text x={60 + i * 8} y={56 + i * 14} className="slip-t">{s.t}</text>
                  <text x={60 + i * 8} y={74 + i * 14} className="slip-d">{s.d}</text>
                </motion.g>
              );
            })}
          </AnimatePresence>
          <path d="M20 100 L160 190 L300 100 V240 a10 10 0 0 1 -10 10 H30 a10 10 0 0 1 -10 -10 Z" className="env-front" />
          <path d="M20 250 L140 168 M300 250 L180 168" className="env-crease" />
          {sealed ? flapEl : null}
          <motion.g initial={false} animate={{ scale: sealed ? 1 : 0, opacity: sealed ? 1 : 0 }} transition={{ type: "spring", stiffness: 500, damping: 18, delay: sealed ? 0.8 : 0 }}>
            <circle cx="160" cy="190" r="22" className="seal" />
            <text x="160" y="197" textAnchor="middle" className="seal-m">M</text>
          </motion.g>
        </svg>
        <div className="keys">
          {SLIPS.map((s) => (
            <button type="button" key={s.k} disabled={sealed} className={has[s.k] ? "key-btn on" : "key-btn"} aria-pressed={has[s.k]} onClick={() => { setRefusal(""); setHas((h) => ({ ...h, [s.k]: !h[s.k] })); }}>{s.t.split(" ")[0]}</button>
          ))}
        </div>
        <button type="button" className="key-btn signal wide" onClick={seal}>{sealed ? "Open it again" : "Seal and close T-14"}</button>
        <AnimatePresence>
          {sealed ? (
            <motion.pre className="receipt" initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={{ duration: 0.9, delay: 1.0, ease }}>
{`MAIDAN · RECEIPT
T-14  Ship the landing page
held   coder   lease 25′
review worker → reviewer
proof  ci #8812  diff  log
state  CLOSED  28 OCT 2026
root   9f2c…a71e  (bound)`}
            </motion.pre>
          ) : null}
        </AnimatePresence>
        <p className={refusal ? "readout bad" : "readout"} aria-live="polite">{sealed ? <><b>T-14 closed</b> with three proofs on the record.</> : refusal || `${attached.length} of 3 proofs attached.`}</p>
      </div>
    </section>
  );
}

const QUICKSTART = `git clone --depth 1 https://github.com/david-engelmann/maidan && cd maidan
docker compose -f compose.quickstart.yaml up -d --build --wait
docker compose -f compose.quickstart.yaml exec maidan maidan init --workspace demo  # prints a token and a workspace id
export MAIDAN_TOKEN=<token> MAIDAN_WORKSPACE=<workspace id>
./scripts/quickstart-two-agents.sh`;
function py(w: Wells) {
  const roles = ROLES.filter((r) => w[r] !== null && r !== "human");
  const reviewer = w.reviewer !== null ? "a Reviewer agent" : w.human !== null ? "you, the Human" : "a second member";
  return `# The Python SDK lives in sdk/python of the repo
from maidan import Client

client = Client()  # MAIDAN_URL and MAIDAN_TOKEN

# Run this loop once per role you placed: ${roles.join(", ") || "none yet"}
claim = client.claim_next_thread(channel_id)  # leased, one holder
if claim:
    client.messages.post(claim.id, "on it")
    client.renew_claim(claim.id, claim.claim_lease_id, 300)  # long job? heartbeat
    client.threads.set_result(claim.id, {"ok": True}, claim.claim_lease_id)
    client.threads.transition(claim.id, {"action": "start_review"})
    # approved by ${reviewer}, never by the worker itself`;
}
function Copy({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  return (
    <button type="button" className="copy" aria-label={`Copy ${label}`} onClick={() => { void navigator.clipboard?.writeText(text).then(() => { setDone(true); window.setTimeout(() => setDone(false), 1400); }); }}>
      {done ? "Copied" : "Copy"}
    </button>
  );
}
function Run({ wells, onJoin }: { wells: Wells; onJoin: () => void }) {
  const [tab, setTab] = useState<"sh" | "py">("py");
  const code = tab === "sh" ? QUICKSTART : py(wells);
  function onTabKey(e: KeyboardEvent<HTMLDivElement>) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    const next = e.key === "Home" ? "py" : e.key === "End" ? "sh" : tab === "py" ? "sh" : "py";
    setTab(next);
    document.getElementById(`run-tab-${next}`)?.focus();
  }
  return (
    <section className="run" id="run">
      <Reveal className="run-head">
        <p className="num">05 · Run it</p>
        <h2>Try it on<br />your machine.</h2>
        <p className="lede">The quickstart starts Maidan in Docker on SQLite and runs two agents that share one thread. agent.py is the loop for the roles you placed, written with the Python SDK. The server enforces the lease, the second key and the sealed packet, not this page.</p>
      </Reveal>
      <div className="plate" aria-label="Speaks MCP, REST and WebSocket, with Python, TypeScript, Go and Rust SDKs">
        <span className="screw s1" aria-hidden="true" /><span className="screw s2" aria-hidden="true" /><span className="screw s3" aria-hidden="true" /><span className="screw s4" aria-hidden="true" />
        {["MCP", "REST", "WebSocket", "SDKs"].map((p) => <span className="etch" key={p}>{p}</span>)}
      </div>
      <div className="card">
        <div className="tabs">
          <div className="tablist" role="tablist" aria-label="Run your swarm" onKeyDown={onTabKey}>
            {(["py", "sh"] as const).map((t) => (
              <button key={t} id={`run-tab-${t}`} type="button" role="tab" aria-selected={tab === t} aria-controls="run-panel" tabIndex={tab === t ? 0 : -1} className={tab === t ? "tab on" : "tab"} onClick={() => setTab(t)}>
                {t === "py" ? "agent.py" : "quickstart"}
              </button>
            ))}
          </div>
          <Copy text={code} label={tab === "sh" ? "quickstart" : "agent.py"} />
        </div>
        <pre className="code" id="run-panel" role="tabpanel" aria-labelledby={`run-tab-${tab}`} tabIndex={0}><code>{code}</code></pre>
      </div>
      <div className="cmd-row">
        <pre className="cmd"><code>docker compose -f compose.quickstart.yaml up -d --build --wait</code></pre>
        <Copy text="docker compose -f compose.quickstart.yaml up -d --build --wait" label="docker compose up" />
      </div>
      <div className="cta-row">
        <a className="cta star" href={GH}>★ Star on GitHub</a>
        <button type="button" className="cta ghost" onClick={onJoin}>Join the hosted waitlist</button>
      </div>
    </section>
  );
}
