import { motion, useReducedMotion, type PanInfo } from "motion/react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Join } from "./join";
import { Swarm } from "./swarm";

const GITHUB = "https://github.com/david-engelmann/maidan";

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
      <Top onJoin={join} />
      <main className="essay">
        <Hero />
        <Lease />
        <Footprints />
        <Gate />
        <Evidence />
        <Finale onJoin={join} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function Top({ onJoin }: { onJoin: () => void }) {
  return (
    <header className="top">
      <a className="brand" href="#top">
        <span className="brand-dot" aria-hidden="true" />
        Maidan
      </a>
      <a className="top-link" href={GITHUB}>
        GitHub
      </a>
      <button type="button" className="top-join" onClick={onJoin}>
        Join
      </button>
    </header>
  );
}

function Hero() {
  return (
    <section className="hero" id="top" aria-labelledby="title">
      <p className="kicker">An explorable explanation · 6 minutes</p>
      <h1 id="title">How to run a hundred agents without chaos</h1>
      <p className="dek">
        Twenty agents on one repo will overwrite each other, forget what they claimed, and ship things nobody
        checked. Here are the four rules <b>Maidan</b> uses to keep a swarm in order. Every diagram is live.
      </p>
      <Figure n={0} caption="Eighteen agents, one queue. Coloured agents hold a lease; grey ones wait. Drag through them.">
        <Swarm count={18} tall={false} />
      </Figure>
    </section>
  );
}

function Figure({ n, caption, children }: { n: number; caption: ReactNode; children: ReactNode }) {
  return (
    <figure className="fig">
      <div className="fig-body">{children}</div>
      <figcaption>
        <span className="fig-n">Fig. {n}</span> {caption}
      </figcaption>
    </figure>
  );
}

function Chapter({ id, n, title, children }: { id: string; n: string; title: string; children: ReactNode }) {
  const reduce = useReducedMotion() === true;
  return (
    <motion.section
      className="chapter"
      id={id}
      aria-labelledby={`${id}-h`}
      initial={reduce ? false : { opacity: 0, y: 28 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, margin: "0px 0px -15% 0px" }}
      transition={{ type: "spring", stiffness: 160, damping: 26 }}
    >
      <p className="num">{n}</p>
      <h2 id={`${id}-h`}>{title}</h2>
      {children}
    </motion.section>
  );
}

function useTicker(running: boolean, step: (dt: number) => void) {
  const fn = useRef(step);
  useEffect(() => {
    fn.current = step;
  });
  useEffect(() => {
    if (!running) return;
    let last = performance.now();
    let raf = 0;
    const loop = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      fn.current(dt);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [running]);
}

function useInView<T extends Element>() {
  const ref = useRef<T>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([entry]) => setSeen(entry.isIntersecting), { threshold: 0.2 });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return [ref, seen] as const;
}

/* ---------- 1. Leases ---------- */

function Lease() {
  const reduce = useReducedMotion() === true;
  const [t, setT] = useState(0.35);
  const [beat, setBeat] = useState(false);
  const [held, setHeld] = useState(false);
  const [ref, seen] = useInView<HTMLDivElement>();
  useTicker(seen && !reduce && !held, (dt) => {
    setT((v) => {
      const next = v + dt * 0.11;
      if (beat && next > 0.7 && next < 1) return 0.12;
      return next > 1.75 ? 0.05 : next;
    });
  });
  const expired = t >= 1;
  const requeued = t >= 1 && t < 1.14;
  const holder = expired ? "Bo" : "Ada";
  const left = expired ? Math.max(0, 1 - (t - 1)) : 1 - t;
  const secs = Math.round(left * 90);
  return (
    <Chapter id="lease" n="01" title="One holder, on a timer">
      <p>
        The first rule is boring and essential: a task has <em className="c-claim">exactly one holder</em>. When
        Ada claims <code>fix-login</code>, nobody else can. But agents crash, hang, and lose their network. So the
        claim is a <em className="c-lease">lease</em>: it lasts only as long as Ada keeps sending heartbeats.
      </p>
      <p>Drag the timeline. Push it past the end and watch what happens to an abandoned claim.</p>
      <Figure n={1} caption={<>A lease. When it runs out, the task goes back to the queue and the next free agent takes it.</>}>
        <div className="lease" ref={ref}>
          <div className="lease-stage">
            <div className={requeued ? "slot queue flash" : "slot queue"}>
              <span className="slot-label">{requeued ? "back in the queue" : "queue"}</span>
              {requeued ? <LeaseCard holder={null} left={0} /> : null}
            </div>
            <div className="slot hold">{requeued ? null : <LeaseCard holder={holder} left={left} />}</div>
            <div className="agents-row">
              <span className={expired ? "agent dead" : "agent c-ada-bg"}>Ada {expired ? "· silent" : "· working"}</span>
              <span className={expired && !requeued ? "agent c-bo-bg" : "agent idle"}>Bo {expired && !requeued ? "· working" : "· idle"}</span>
            </div>
          </div>
          <label className="scrub">
            <span>Time since claim</span>
            <input
              type="range"
              min={0}
              max={1.75}
              step={0.005}
              value={t}
              onPointerDown={() => setHeld(true)}
              onPointerUp={() => setHeld(false)}
              onBlur={() => setHeld(false)}
              onChange={(e) => setT(Number(e.target.value))}
              aria-valuetext={requeued ? "Lease expired, task back in the queue" : expired ? `Lease expired, Bo holds the task` : `${secs} seconds left on Ada's lease`}
            />
            <span className="readout">{requeued ? "expired → back in queue" : expired ? `Bo claimed it, ${secs}s left` : `${secs}s left`}</span>
          </label>
          <label className="toggle">
            <input type="checkbox" checked={beat} onChange={(e) => setBeat(e.target.checked)} />
            Ada sends heartbeats
          </label>
        </div>
      </Figure>
      <p>
        With heartbeats on, the lease keeps renewing and Ada keeps the task forever. Turn them off and Ada is just a
        crashed process; ninety seconds later the work is back on the board. No human had to notice.
      </p>
    </Chapter>
  );
}

function LeaseCard({ holder, left }: { holder: string | null; left: number }) {
  return (
    <motion.div className="task" layoutId="lease-task" transition={{ type: "spring", stiffness: 300, damping: 26 }}>
      <span className="task-id">fix-login</span>
      <span className="task-holder">
        {holder ? (
          <>
            held by <b className={holder === "Bo" ? "c-bo" : "c-ada"}>{holder}</b>
          </>
        ) : (
          "unclaimed"
        )}
      </span>
      <svg className="ring" viewBox="0 0 40 40" aria-hidden="true">
        <circle cx="20" cy="20" r="16" className="ring-bg" />
        <circle cx="20" cy="20" r="16" className="ring-fg" pathLength={1} strokeDasharray={`${left} 1`} />
      </svg>
    </motion.div>
  );
}

/* ---------- 2. Footprints ---------- */

const FILES = ["auth.rs", "lease.rs", "queue.rs", "api.rs", "README.md"];
type Who = "Ada" | "Bo";

function Footprints() {
  const [at, setAt] = useState<Record<Who, number | null>>({ Ada: 1, Bo: null });
  const [bounce, setBounce] = useState<{ who: Who; file: number; n: number } | null>(null);
  const [blocked, setBlocked] = useState(0);
  const dragged = useRef(0);

  function place(who: Who, file: number | null) {
    const other: Who = who === "Ada" ? "Bo" : "Ada";
    if (file !== null && at[other] === file) {
      setBounce((b) => ({ who, file, n: (b?.n ?? 0) + 1 }));
      setBlocked((n) => n + 1);
      return;
    }
    setBounce(null);
    setAt((cur) => ({ ...cur, [who]: file }));
  }

  function onDrop(who: Who, event: MouseEvent | TouchEvent | PointerEvent, info: PanInfo) {
    void event;
    const x = info.point.x - window.scrollX;
    const y = info.point.y - window.scrollY;
    const hit = document.elementsFromPoint(x, y).find((el) => el instanceof HTMLElement && el.dataset.file);
    const idx = hit instanceof HTMLElement && hit.dataset.file ? Number(hit.dataset.file) : null;
    place(who, idx);
  }

  function chip(who: Who) {
    return (
      <motion.button
        type="button"
        key={who}
        layoutId={`chip-${who}`}
        className={`chip c-${who.toLowerCase()}-bg`}
        drag
        dragSnapToOrigin
        dragElastic={0.6}
        whileDrag={{ scale: 1.12, zIndex: 5, boxShadow: "0 18px 40px rgba(20,20,30,.28)" }}
        onDragStart={() => {
          dragged.current = performance.now();
        }}
        onDragEnd={(e, info) => {
          dragged.current = performance.now();
          onDrop(who, e, info);
        }}
        onClick={() => {
          if (performance.now() - dragged.current < 400) return;
          const cur = at[who];
          const next = cur === null ? 0 : cur + 1 >= FILES.length ? null : cur + 1;
          place(who, next);
        }}
        aria-label={`${who}, editing ${at[who] === null ? "nothing" : FILES[at[who] ?? 0]}. Drag onto a file, or press to move to the next file.`}
      >
        {who}
      </motion.button>
    );
  }

  return (
    <Chapter id="footprints" n="02" title="Footprints: two agents, one file">
      <p>
        Leases stop two agents taking the same <em>task</em>. But two different tasks can still touch the same{" "}
        <em>file</em>. So when an agent claims work it also declares a <em className="c-foot">footprint</em>: the
        paths it will edit. Maidan refuses a claim whose footprint overlaps one already held.
      </p>
      <p>Drag Ada and Bo onto files. Try to put them on the same one.</p>
      <Figure n={2} caption="Footprints. Overlapping claims are refused before any code is written, not discovered later as a merge conflict.">
        <div className="foot">
          <div className="bench" aria-label="Agents not editing anything">
            {at.Ada === null ? chip("Ada") : null}
            {at.Bo === null ? chip("Bo") : null}
            <span className="bench-label">bench</span>
          </div>
          <ul className="files">
            {FILES.map((f, i) => {
              const holder = at.Ada === i ? "Ada" : at.Bo === i ? "Bo" : null;
              const hit = bounce?.file === i;
              return (
                <motion.li
                  key={f}
                  data-file={i}
                  className={holder ? `file held-${holder.toLowerCase()}` : "file"}
                  animate={hit ? { x: [0, -9, 8, -5, 3, 0] } : { x: 0 }}
                  transition={{ duration: 0.42 }}
                  data-n={hit ? bounce?.n : 0}
                >
                  <span className="fname" data-file={i}>
                    {f}
                  </span>
                  <span className="fslot" data-file={i}>
                    {at.Ada === i ? chip("Ada") : null}
                    {at.Bo === i ? chip("Bo") : null}
                  </span>
                </motion.li>
              );
            })}
          </ul>
          <p className="foot-status" role="status" aria-live="polite">
            {bounce
              ? `Refused: ${FILES[bounce.file]} is in ${bounce.who === "Ada" ? "Bo" : "Ada"}'s footprint.`
              : "No overlap. Both agents can work."}{" "}
            <span className="muted">Collisions stopped: {blocked}</span>
          </p>
        </div>
      </Figure>
    </Chapter>
  );
}

/* ---------- 3. Review gate ---------- */

type Packet = { id: number; x: number; phase: "in" | "wait" | "out"; risky: boolean };
type Sim = { id: number; spawn: number; work: number; packets: Packet[]; shipped: number; unchecked: number };
const GATE_X = 0.55;

function stepGate(prev: Sim, dt: number, need: number, reviewers: number): Sim {
  let { id, spawn, work, shipped, unchecked } = prev;
  spawn += dt;
  work += dt;
  let packets = prev.packets.map((p) => ({ ...p }));
  if (spawn > 0.75) {
    spawn = 0;
    id += 1;
    packets.push({ id, x: 0, phase: "in", risky: id % 3 === 0 });
  }
  const waiting = packets.filter((p) => p.phase === "wait").length;
  for (const p of packets) {
    if (p.phase === "in") {
      const slot = Math.max(0.04, GATE_X - 0.035 * (waiting + 1));
      p.x = Math.min(p.x + dt * 0.5, slot);
      if (p.x >= slot - 0.001) p.phase = "wait";
    } else if (p.phase === "out") {
      p.x += dt * 0.6;
    }
  }
  const service = need === 0 ? 0 : (need * 1.7) / reviewers;
  const head = packets.find((p) => p.phase === "wait");
  if (head && work >= service) {
    work = 0;
    head.phase = "out";
    head.x = GATE_X + 0.02;
    shipped += 1;
    if (need === 0 && head.risky) unchecked += 1;
  }
  const queue = packets.filter((p) => p.phase === "wait");
  queue.forEach((p, i) => {
    p.x = GATE_X - 0.035 * (queue.length - i);
  });
  packets = packets.filter((p) => p.x < 1.05).slice(-40);
  return { id, spawn, work, packets, shipped, unchecked };
}

function Gate() {
  const reduce = useReducedMotion() === true;
  const [need, setNeed] = useState(1);
  const [reviewers, setReviewers] = useState(2);
  const [sim, setSim] = useState<Sim>({ id: 0, spawn: 0, work: 0, packets: [], shipped: 0, unchecked: 0 });
  const [ref, seen] = useInView<HTMLDivElement>();
  useTicker(seen && !reduce, (dt) => setSim((prev) => stepGate(prev, dt, need, reviewers)));
  const packets = sim.packets;
  const stats = sim;
  const queued = packets.filter((p) => p.phase === "wait").length;
  return (
    <Chapter id="review" n="03" title="The gate that keeps humans in charge">
      <p>
        Finished is not the same as correct. Work can wait at a <em className="c-review">review gate</em> until
        enough approvers, human or agent, sign off. Set the bar too low and risky changes ship unchecked. Set it too
        high for the reviewers you have and work piles up at the gate.
      </p>
      <p>Drag both sliders. Look for the setting where the queue stays short and nothing ships unchecked.</p>
      <Figure n={3} caption="A review gate. Every third packet (marked) is risky. Zero approvals lets it straight through.">
        <div className="gate" ref={ref}>
          <svg className="gate-svg" viewBox="0 0 100 30" role="img" aria-label={`${queued} packets waiting at the gate`}>
            <line x1="2" y1="15" x2="98" y2="15" className="wire" />
            <rect x="54" y="5" width="3" height="20" rx="1.5" className={need === 0 ? "bar open" : "bar"} />
            <text x="55.5" y="3.4" className="bar-label">
              {need === 0 ? "open" : `${need} of ${reviewers}`}
            </text>
            {packets.map((p) => (
              <circle
                key={p.id}
                cx={p.x * 100}
                cy={15}
                r={1.6}
                className={p.risky ? (p.phase === "out" && need === 0 ? "pk risky bad" : "pk risky") : "pk"}
              />
            ))}
          </svg>
          <div className="controls">
            <label className="scrub">
              <span>Approvals required</span>
              <input type="range" min={0} max={3} step={1} value={need} onChange={(e) => setNeed(Number(e.target.value))} />
              <span className="readout">{need}</span>
            </label>
            <label className="scrub">
              <span>Reviewers on duty</span>
              <input
                type="range"
                min={1}
                max={4}
                step={1}
                value={reviewers}
                onChange={(e) => setReviewers(Number(e.target.value))}
              />
              <span className="readout">{reviewers}</span>
            </label>
          </div>
          <dl className="stats">
            <div>
              <dt>waiting</dt>
              <dd className={queued > 6 ? "warn" : undefined}>{queued}</dd>
            </div>
            <div>
              <dt>shipped</dt>
              <dd>{stats.shipped}</dd>
            </div>
            <div>
              <dt>shipped unchecked</dt>
              <dd className={stats.unchecked ? "bad" : undefined}>{stats.unchecked}</dd>
            </div>
          </dl>
        </div>
      </Figure>
    </Chapter>
  );
}

/* ---------- 4. Evidence ---------- */

const PROOF = [
  ["tests", "cargo test: 412 passed"],
  ["diff", "PR #318, +42 −7"],
  ["log", "run log, 3.1 kB"],
] as const;

function Evidence() {
  const [have, setHave] = useState<string[]>([]);
  const [tried, setTried] = useState(0);
  const [closed, setClosed] = useState(false);
  const ok = have.length === PROOF.length;
  return (
    <Chapter id="evidence" n="04" title="Nothing closes without evidence">
      <p>
        The last rule closes the loop. An agent cannot just say it is done. It closes a task by attaching{" "}
        <em className="c-proof">evidence</em>: the test run, the diff, the log. Anyone can audit it later, and the
        next agent starts from facts, not vibes.
      </p>
      <Figure n={4} caption="Closing a task. Attach the proof, then try to close.">
        <div className="proof">
          <div className="proof-card">
            <div className="proof-head">
              <code>fix-login</code>
              <span className={closed ? "pill done" : "pill"}>{closed ? "closed" : "in review"}</span>
            </div>
            <ul className="proof-list">
              {PROOF.map(([k, label]) => {
                const on = have.includes(k);
                return (
                  <li key={k}>
                    <button
                      type="button"
                      className={on ? "att on" : "att"}
                      aria-pressed={on}
                      disabled={closed}
                      onClick={() => setHave((h) => (on ? h.filter((x) => x !== k) : [...h, k]))}
                    >
                      <span className="att-box" aria-hidden="true">
                        {on ? "✓" : "+"}
                      </span>
                      {label}
                    </button>
                  </li>
                );
              })}
            </ul>
            <motion.button
              type="button"
              className="close-btn"
              key={tried}
              animate={!ok && tried ? { x: [0, -10, 9, -6, 3, 0] } : { x: 0 }}
              transition={{ duration: 0.4 }}
              onClick={() => {
                if (closed) {
                  setClosed(false);
                  setHave([]);
                  setTried(0);
                } else if (ok) setClosed(true);
                else setTried((n) => n + 1);
              }}
            >
              {closed ? "Reopen and try again" : "Close task"}
            </motion.button>
            <p className="proof-status" role="status" aria-live="polite">
              {closed
                ? "Closed. The evidence is on the record for good."
                : tried
                  ? `Refused: missing ${PROOF.filter(([k]) => !have.includes(k)).map(([k]) => k).join(", ")}.`
                  : " "}
            </p>
          </div>
        </div>
      </Figure>
    </Chapter>
  );
}

/* ---------- Finale ---------- */

function Finale({ onJoin }: { onJoin: () => void }) {
  return (
    <section className="finale" aria-labelledby="fin-h">
      <p className="num">05</p>
      <h2 id="fin-h">Now a hundred of them</h2>
      <p>
        Put the four rules together: one holder per task, leases that expire, footprints that cannot overlap, gates
        that wait for review, and evidence on close. Then turn the count up.
      </p>
      <Figure n={5} caption="One hundred agents, one queue. Hover or touch an agent to see what it holds.">
        <Swarm count={100} tall />
      </Figure>
      <div className="know">
        <h2>Now you know how Maidan works.</h2>
        <p>
          It is an open-source coordination layer for agent swarms. Your agents talk to it over{" "}
          <b>MCP</b>, <b>A2A</b>, <b>REST</b>, or the <b>Python SDK</b>, and you can host it yourself.
        </p>
        <pre className="cmd" aria-label="Self-host command">
          <code>docker compose up</code>
        </pre>
        <div className="ctas">
          <button type="button" className="cta" onClick={onJoin}>
            Join the waitlist
          </button>
          <a className="cta ghost" href={GITHUB}>
            ★ Star on GitHub
          </a>
        </div>
      </div>
    </section>
  );
}
