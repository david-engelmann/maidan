import { motion, useReducedMotion, type PanInfo } from "motion/react";
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent as RPointerEvent, type ReactNode } from "react";
import { FILM, runFilm, useFilm } from "./film";
import { Hero } from "./hero";
import { Join } from "./join";
import { Swarm } from "./swarm";

const GITHUB = "https://github.com/david-engelmann/maidan";
const ease = [0.22, 1, 0.36, 1] as const;

export default function App() {
  const [open, setOpen] = useState(false);
  const [review, setReview] = useState(false);
  useEffect(() => {
    const openIf = () => {
      if (location.hash === "#join") setOpen(true);
    };
    openIf();
    window.addEventListener("hashchange", openIf);
    if (FILM) void runFilm();
    const fin = document.getElementById("finale");
    const io = new IntersectionObserver(([e]) => document.documentElement.classList.toggle("night", e.isIntersecting), { rootMargin: "0px 0px -92% 0px" });
    if (fin) io.observe(fin);
    return () => {
      window.removeEventListener("hashchange", openIf);
      io.disconnect();
    };
  }, []);
  const join = () => setOpen(true);
  return (
    <>
      <Top onJoin={join} />
      <main className="essay">
        <Hero film={FILM} onPlaced={setReview} />
        <Lease />
        <Footprints />
        <Gate />
        <Evidence />
      </main>
      <Finale onJoin={join} review={review} />
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
        <span aria-hidden="true">★</span> Star
      </a>
      <button type="button" className="top-join" onClick={onJoin}>
        Join
      </button>
    </header>
  );
}

function Figure({ n, caption, hint, children }: { n: number; caption: ReactNode; hint: string; children: ReactNode }) {
  return (
    <figure className="fig">
      <div className="fig-body">
        <span className="touch" aria-hidden="true">
          <i />
          {hint}
        </span>
        {children}
      </div>
      <figcaption>
        <span className="fig-n">Fig. {n}</span> {caption}
      </figcaption>
    </figure>
  );
}

function Chapter({ id, n, title, children }: { id: string; n: string; title: ReactNode; children: ReactNode }) {
  const reduce = useReducedMotion() === true;
  return (
    <motion.section
      className="chapter"
      id={id}
      aria-labelledby={`${id}-h`}
      initial={reduce ? false : { opacity: 0, y: 28 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, margin: "0px 0px -10% 0px" }}
      transition={{ duration: 0.7, ease }}
    >
      <p className="num">Rule {n}</p>
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
    const io = new IntersectionObserver(([entry]) => setSeen(entry.isIntersecting), { threshold: 0.15 });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return [ref, seen] as const;
}

/* ---------- 1. Leases: a dial ---------- */

const T_MAX = 1.75;
const LEASE_S = 90;

function Lease() {
  const reduce = useReducedMotion() === true;
  const [t, setT] = useState(0.35);
  const [beat, setBeat] = useState(false);
  const [held, setHeld] = useState(false);
  const [ref, seen] = useInView<HTMLDivElement>();
  const dial = useRef<SVGSVGElement>(null);
  useTicker(seen && !reduce && !held, (dt) => {
    setT((v) => {
      const next = v + dt * (FILM ? 0.26 : 0.11);
      if (beat && next > 0.7 && next < 1) return 0.12;
      return next > T_MAX ? 0.05 : next;
    });
  });
  useFilm("lease", () => {
    setBeat(false);
    setT(0.55);
  });
  const expired = t >= 1;
  const requeued = t >= 1 && t < 1.14;
  const holder = expired ? "Bo" : "Ada";
  const left = expired ? Math.max(0, 1 - (t - 1)) : 1 - t;
  const secs = Math.round(left * LEASE_S);
  const angle = (t / T_MAX) * 300 - 150;
  const text = requeued ? "Lease expired, task back in the queue" : expired ? `Lease expired, Bo holds the task, ${secs} seconds left` : `${secs} seconds left on Ada's lease`;

  function fromPointer(e: RPointerEvent<SVGSVGElement>) {
    const r = dial.current?.getBoundingClientRect();
    if (!r) return;
    const a = (Math.atan2(e.clientX - (r.left + r.width / 2), -(e.clientY - (r.top + r.height / 2))) * 180) / Math.PI;
    setT(((Math.max(-150, Math.min(150, a)) + 150) / 300) * T_MAX);
  }
  function onKey(e: KeyboardEvent<SVGSVGElement>) {
    const map: Record<string, number> = { ArrowUp: 0.05, ArrowRight: 0.05, ArrowDown: -0.05, ArrowLeft: -0.05, PageUp: 0.2, PageDown: -0.2 };
    if (e.key in map) {
      e.preventDefault();
      setT((v) => Math.max(0, Math.min(T_MAX, v + map[e.key])));
    } else if (e.key === "Home") setT(0);
    else if (e.key === "End") setT(T_MAX);
  }

  return (
    <Chapter id="lease" n="01" title={<>One holder,<br />on a timer.</>}>
      <p>
        A task has <em className="c-claim">exactly one holder</em>. The claim is a <em className="c-lease">lease</em>: it lasts only while the
        agent keeps sending heartbeats.
      </p>
      <Figure n={1} hint="Turn the dial" caption="A lease. Turn past the red mark and the abandoned task goes back to the queue.">
        <div className="lease" ref={ref}>
          <svg
            ref={dial}
            className="dial"
            viewBox="0 0 240 240"
            role="slider"
            tabIndex={0}
            aria-label="Time since Ada claimed fix-login"
            aria-valuemin={0}
            aria-valuemax={Math.round(T_MAX * LEASE_S)}
            aria-valuenow={Math.round(t * LEASE_S)}
            aria-valuetext={text}
            onKeyDown={onKey}
            onPointerDown={(e) => {
              setHeld(true);
              e.currentTarget.setPointerCapture(e.pointerId);
              fromPointer(e);
            }}
            onPointerMove={(e) => {
              if (held) fromPointer(e);
            }}
            onPointerUp={() => setHeld(false)}
            onPointerCancel={() => setHeld(false)}
          >
            <defs>
              <radialGradient id="knob-g" cx="0.4" cy="0.3" r="0.8">
                <stop offset="0" stopColor="#ffffff" />
                <stop offset="0.6" stopColor="#e2e0da" />
                <stop offset="1" stopColor="#bdbab2" />
              </radialGradient>
              <linearGradient id="face-g" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0" stopColor="#f6f5f1" />
                <stop offset="1" stopColor="#d9d6ce" />
              </linearGradient>
            </defs>
            {Array.from({ length: 61 }, (_, i) => {
              const a = ((i / 60) * 300 - 150) * (Math.PI / 180);
              const long = i % 5 === 0;
              const r1 = long ? 99 : 104;
              const v = (i / 60) * T_MAX;
              const cls = Math.abs(v - 1) < 0.015 ? "tick end" : v <= t ? "tick on" : "tick";
              return <line key={i} x1={120 + Math.sin(a) * r1} y1={120 - Math.cos(a) * r1} x2={120 + Math.sin(a) * 112} y2={120 - Math.cos(a) * 112} className={cls} />;
            })}
            <circle cx="120" cy="126" r="84" className="knob-shadow" />
            <circle cx="120" cy="120" r="82" fill="url(#knob-g)" className="knob" />
            <circle cx="120" cy="120" r="66" fill="url(#face-g)" className="knob-face" />
            <circle cx="120" cy="120" r="56" className="lease-ring-bg" />
            <circle
              cx="120"
              cy="120"
              r="56"
              className={requeued ? "lease-ring gone" : "lease-ring"}
              pathLength={1}
              strokeDasharray={`${requeued ? 0 : left} 1`}
              transform="rotate(-90 120 120)"
            />
            <g transform={`rotate(${angle} 120 120)`}>
              <rect x="117" y="40" width="6" height="22" rx="3" className="notch" />
            </g>
            <text x="120" y="118" textAnchor="middle" className="dial-read">
              {requeued ? "—" : `${secs}s`}
            </text>
            <text x="120" y="140" textAnchor="middle" className="dial-sub">
              {requeued ? "requeued" : `${holder}'s lease`}
            </text>
          </svg>
          <div className="lease-side">
            <div className={requeued ? "slot queue flash" : "slot queue"}>
              <span className="slot-label">{requeued ? "back in the queue" : "queue"}</span>
            </div>
            <div className={`ticket${requeued ? " loose" : ""}`}>
              <span className="task-id">fix-login</span>
              <span className="task-holder">
                {requeued ? "unclaimed" : <>held by <b className={holder === "Bo" ? "c-bo" : "c-ada"}>{holder}</b></>}
              </span>
            </div>
            <div className="agents-row">
              <span className={expired ? "agent dead" : "agent c-ada-bg"}>Ada {expired ? "· silent" : "· working"}</span>
              <span className={expired && !requeued ? "agent c-bo-bg" : "agent idle"}>Bo {expired && !requeued ? "· working" : "· idle"}</span>
            </div>
            <button type="button" role="switch" aria-checked={beat} className={`switch light${beat ? " on" : ""}`} onClick={() => setBeat((b) => !b)}>
              <span className="switch-track" aria-hidden="true"><span className="switch-knob" /></span>
              Ada sends heartbeats
            </button>
            <p className="readout" aria-live="polite">{requeued ? "Expired. Back on the board, no human needed." : expired ? `Bo picked it up. ${secs}s left.` : beat ? "Heartbeats renew the lease." : `${secs}s left on Ada's lease.`}</p>
          </div>
        </div>
      </Figure>
      <p>Heartbeats on, Ada keeps it. Off, Ada is a crashed process, and ninety seconds later the work is back on the board.</p>
    </Chapter>
  );
}

/* ---------- 2. Footprints: a steel tile ---------- */

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
  useFilm("footprints", () => {
    place("Bo", 3);
    window.setTimeout(() => place("Bo", 1), 1600);
  });

  function onDrop(who: Who, info: PanInfo) {
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
        className={`puck puck-${who.toLowerCase()}`}
        drag
        dragSnapToOrigin
        dragElastic={0.6}
        whileDrag={{ scale: 1.15, zIndex: 5 }}
        onDragStart={() => {
          dragged.current = performance.now();
        }}
        onDragEnd={(_e, info) => {
          dragged.current = performance.now();
          onDrop(who, info);
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
    <Chapter id="footprints" n="02" title={<>Two agents.<br />Never one file.</>}>
      <p>
        Every claim declares a <em className="c-foot">footprint</em>, the files it will edit. Maidan refuses a claim that overlaps one already held.
      </p>
      <Figure n={2} hint="Drag the pucks" caption="Footprints. Overlaps are refused before any code is written, not found later as a merge conflict.">
        <div className="steel">
          <span className="screw s1" /><span className="screw s2" /><span className="screw s3" /><span className="screw s4" />
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
                  className={holder ? `file held-${holder.toLowerCase()}${hit ? " refused" : ""}` : "file"}
                  animate={hit ? { x: [0, -9, 8, -5, 3, 0] } : { x: 0 }}
                  transition={{ duration: 0.42 }}
                  data-n={hit ? bounce?.n : 0}
                >
                  <span className="fname" data-file={i}>{f}</span>
                  <span className="fslot" data-file={i}>
                    {at.Ada === i ? chip("Ada") : null}
                    {at.Bo === i ? chip("Bo") : null}
                  </span>
                </motion.li>
              );
            })}
          </ul>
          <p className="engrave-line">MAIDAN · FOOTPRINT BENCH</p>
        </div>
        <p className="foot-status" role="status" aria-live="polite">
          {bounce ? <><b className="c-danger">Refused:</b> {FILES[bounce.file]} is in {bounce.who === "Ada" ? "Bo" : "Ada"}'s footprint.</> : "No overlap. Both agents can work."}{" "}
          <span className="muted">Collisions stopped: {blocked}</span>
        </p>
      </Figure>
    </Chapter>
  );
}

/* ---------- 3. Review gate: a padlock ---------- */

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
    } else if (p.phase === "out") p.x += dt * 0.6;
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

function Stepper({ label, value, min, max, onChange }: { label: string; value: number; min: number; max: number; onChange: (v: number) => void }) {
  return (
    <div className="stepper" role="group" aria-label={label}>
      <span className="stepper-l">{label}</span>
      <button type="button" aria-label={`Fewer: ${label}`} disabled={value <= min} onClick={() => onChange(value - 1)}>−</button>
      <output aria-live="polite">{value}</output>
      <button type="button" aria-label={`More: ${label}`} disabled={value >= max} onClick={() => onChange(value + 1)}>+</button>
    </div>
  );
}

function Gate() {
  const reduce = useReducedMotion() === true;
  const [need, setNeed] = useState(2);
  const [reviewers, setReviewers] = useState(2);
  const [sim, setSim] = useState<Sim>({ id: 0, spawn: 0, work: 0, packets: [], shipped: 0, unchecked: 0 });
  const [ref, seen] = useInView<HTMLDivElement>();
  useTicker(seen && !reduce, (dt) => setSim((prev) => stepGate(prev, dt, need, reviewers)));
  useFilm("review", () => {
    setNeed(0);
    window.setTimeout(() => setNeed(2), 2600);
  });
  const queued = sim.packets.filter((p) => p.phase === "wait").length;
  const service = need === 0 ? 0 : (need * 1.7) / reviewers;
  const turned = need === 0 || !queued ? 0 : Math.min(need, Math.floor((sim.work / service) * need));
  const released = need === 0 || sim.work < 0.3;
  const holes = Math.max(need, 1);
  return (
    <Chapter id="review" n="03" title={<>Two keys<br />to ship.</>}>
      <p>
        Finished is not correct. Work waits at a <em className="c-review">review gate</em> until enough approvers, human or agent, turn their keys.
      </p>
      <Figure n={3} hint="Change the keys" caption="A review gate. Every third packet is risky. With zero keys it walks straight through.">
        <div className="gate" ref={ref}>
          <svg className="padlock" viewBox="0 0 280 230" role="img" aria-label={need === 0 ? "Padlock removed: nothing is reviewed" : `Padlock needing ${need} keys, ${turned} turned`}>
            <defs>
              <linearGradient id="lock-g" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0" stopColor="#2b2a27" />
                <stop offset="1" stopColor="#121110" />
              </linearGradient>
              <linearGradient id="shackle-g" x1="0" y1="0" x2="1" y2="0">
                <stop offset="0" stopColor="#8f8d87" />
                <stop offset="0.5" stopColor="#f2f1ed" />
                <stop offset="1" stopColor="#8f8d87" />
              </linearGradient>
            </defs>
            <motion.path
              d="M88 104 V74 a52 52 0 0 1 104 0 V104"
              className="shackle"
              stroke="url(#shackle-g)"
              initial={false}
              animate={{ y: released ? -22 : 0, opacity: need === 0 ? 0.25 : 1 }}
              transition={{ type: "spring", stiffness: 300, damping: 16 }}
            />
            <rect x="40" y="96" width="200" height="128" rx="26" fill="url(#lock-g)" className="body" />
            <rect x="40.5" y="96.5" width="199" height="127" rx="26" className="body-sheen" />
            <text x="140" y="124" textAnchor="middle" className="engrave">
              {need === 0 ? "NO REVIEW" : released ? "RELEASED" : "LOCKED"}
            </text>
            {Array.from({ length: holes }, (_, i) => {
              const cx = 140 + (i - (holes - 1) / 2) * 62;
              const on = need > 0 && i < turned;
              return (
                <g key={i} className={need === 0 ? "hole off" : "hole"}>
                  <circle cx={cx} cy="172" r="19" className="keyhole-plate" />
                  <path d={`M${cx - 4} 166 a6 6 0 1 1 8 0 l3 15 h-14 z`} className="keyhole" />
                  <g transform={`translate(${cx} 172)`}>
                    <motion.g initial={false} animate={{ opacity: on ? 1 : 0, rotate: on ? 90 : 0, y: on ? 0 : 18 }} transition={{ duration: 0.35, ease }}>
                      <rect x="-3" y="-16" width="6" height="16" rx="2" className="keybit" />
                      <circle cx="0" cy="-23" r="9" className="keybow" />
                    </motion.g>
                  </g>
                </g>
              );
            })}
          </svg>
          <svg className="gate-svg" viewBox="0 0 100 14" role="img" aria-label={`${queued} packets waiting at the gate`}>
            <line x1="2" y1="7" x2="98" y2="7" className="wire" />
            <rect x="54" y="1" width="3" height="12" rx="1.5" className={need === 0 ? "bar open" : "bar"} />
            {sim.packets.map((p) => (
              <circle key={p.id} cx={p.x * 100} cy={7} r={1.7} className={p.risky ? (p.phase === "out" && need === 0 ? "pk risky bad" : "pk risky") : "pk"} />
            ))}
          </svg>
          <div className="controls">
            <Stepper label="Keys required" value={need} min={0} max={3} onChange={setNeed} />
            <Stepper label="Reviewers on duty" value={reviewers} min={1} max={4} onChange={setReviewers} />
          </div>
          <dl className="stats" aria-live="polite">
            <div><dt>waiting</dt><dd className={queued > 6 ? "warn" : undefined}>{queued}</dd></div>
            <div><dt>shipped</dt><dd>{sim.shipped}</dd></div>
            <div><dt>unchecked</dt><dd className={sim.unchecked ? "bad" : undefined}>{sim.unchecked}</dd></div>
          </dl>
        </div>
      </Figure>
      <p>Too few keys and risky work ships. Too many for your reviewers and it piles up at the gate.</p>
    </Chapter>
  );
}

/* ---------- 4. Evidence: a sealed envelope ---------- */

const PROOF = [
  ["tests", "CI run #8812", "412 passed"],
  ["diff", "Diff", "PR #318, +42 −7"],
  ["log", "Run log", "3.1 kB, signed"],
] as const;

function Evidence() {
  const [have, setHave] = useState<string[]>([]);
  const [tried, setTried] = useState(0);
  const [closed, setClosed] = useState(false);
  const ok = have.length === PROOF.length;
  function close() {
    if (closed) {
      setClosed(false);
      setHave([]);
      setTried(0);
    } else if (ok) setClosed(true);
    else setTried((n) => n + 1);
  }
  useFilm("evidence", () => {
    setClosed(false);
    setHave([]);
    setTried((n) => n + 1);
    PROOF.forEach(([k], i) => window.setTimeout(() => setHave((h) => (h.includes(k) ? h : [...h, k])), 1300 + i * 550));
    window.setTimeout(() => setClosed(true), 3500);
  });
  return (
    <Chapter id="evidence" n="04" title={<>Closed means<br />proven.</>}>
      <p>
        An agent can't just say it's done. It closes a task by sealing in <em className="c-proof">evidence</em>: the test run, the diff, the log.
      </p>
      <Figure n={4} hint="Attach, then close" caption="Closing a task. Try Close first. Maidan refuses until the proof is inside.">
        <div className="proof">
          <svg className="envelope" viewBox="0 0 320 250" role="img" aria-label={closed ? "Envelope sealed with three proofs inside" : `Open envelope, ${have.length} of 3 proofs inside`}>
            <rect x="20" y="80" width="280" height="160" rx="12" className="env-back" />
            {PROOF.map(([k, t, d], i) => {
              const inside = have.includes(k);
              return (
                <motion.g key={k} initial={false} animate={{ y: inside ? 78 - i * 4 : -i * 4 }} transition={{ duration: 0.55, ease }}>
                  <rect x={44 + i * 10} y={18 + i * 16} width={232 - i * 20} height="86" rx="7" className="slip" />
                  <text x={58 + i * 10} y={42 + i * 16} className="slip-t">{t}</text>
                  <text x={58 + i * 10} y={58 + i * 16} className="slip-d">{d}</text>
                </motion.g>
              );
            })}
            <path d="M20 92 L160 180 L300 92 V228 a12 12 0 0 1 -12 12 H32 a12 12 0 0 1 -12 -12 Z" className="env-front" />
            <path d="M20 240 L140 160 M300 240 L180 160" className="env-crease" />
            <motion.path
              d="M20 92 L160 186 L300 92 Z"
              className="flap"
              initial={false}
              animate={{ scaleY: closed ? 1 : -1 }}
              transition={{ duration: 0.5, delay: closed ? 0.15 : 0, ease }}
              style={{ originX: 0.5, originY: 0 }}
            />
            <motion.g initial={false} animate={{ scale: closed ? 1 : 0, opacity: closed ? 1 : 0 }} transition={{ type: "spring", stiffness: 500, damping: 16, delay: closed ? 0.55 : 0 }}>
              <circle cx="160" cy="182" r="24" className="seal" />
              <circle cx="160" cy="182" r="17" className="seal-ring" />
              <text x="160" y="189" textAnchor="middle" className="seal-m">M</text>
            </motion.g>
          </svg>
          <ul className="proof-list">
            {PROOF.map(([k, label]) => {
              const on = have.includes(k);
              return (
                <li key={k}>
                  <button type="button" className={on ? "att on" : "att"} aria-pressed={on} disabled={closed} onClick={() => setHave((h) => (on ? h.filter((x) => x !== k) : [...h, k]))}>
                    <span className="att-box" aria-hidden="true">{on ? "✓" : "+"}</span>
                    {label}
                  </button>
                </li>
              );
            })}
          </ul>
          <motion.button type="button" className="close-btn" key={tried} animate={!ok && tried ? { x: [0, -10, 9, -6, 3, 0] } : { x: 0 }} transition={{ duration: 0.4 }} onClick={close}>
            {closed ? "Reopen and try again" : "Close task"}
          </motion.button>
          <p className="proof-status" role="status" aria-live="polite">
            {closed ? <><b className="c-proof">Closed.</b> Sealed and on the record for good.</> : tried ? <><b className="c-danger">Refused:</b> missing {PROOF.filter(([k]) => !have.includes(k)).map(([k]) => k).join(", ")}.</> : " "}
          </p>
        </div>
      </Figure>
    </Chapter>
  );
}

/* ---------- Finale ---------- */

function yaml(review: boolean) {
  return [
    "# maidan.yaml",
    "square: my-swarm",
    "agents:",
    "  - role: coder",
    "    count: 96",
    "  - role: reviewer",
    `    count: ${review ? 4 : 0}`,
    "rules:",
    "  leases: { ttl: 90s, heartbeat: 15s }",
    "  footprints: required",
    `  review_gate: ${review ? "{ min_approvals: 2 }" : "off"}`,
    "  evidence: [tests, diff, log]",
  ].join("\n");
}
const PY = [
  "from maidan import Square",
  "",
  'sq = Square.connect("my-swarm")',
  'agent = sq.join(role="coder")',
  "",
  "for task in agent.tasks():",
  "    with agent.lease(task, ttl=90):",
  '        agent.declare(task, files=["src/auth/**"])',
  "        result = do_work(task)",
  "        agent.close(task, evidence=result.proof)",
].join("\n");

function Copy({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="copy"
      aria-label={`Copy ${label}`}
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setDone(true);
          window.setTimeout(() => setDone(false), 1400);
        });
      }}
    >
      {done ? "Copied ✓" : "Copy"}
    </button>
  );
}

function Finale({ onJoin, review }: { onJoin: () => void; review: boolean }) {
  const reduce = useReducedMotion() === true;
  const y = yaml(review);
  return (
    <section className="finale" id="finale" aria-labelledby="fin-h">
      <div className="fin-inner">
        <p className="num">All four rules</p>
        <h2 id="fin-h">Now pull back.<br /><span className="h1-soft">A hundred of them.</span></h2>
        <motion.div
          className="fin-stage"
          initial={reduce ? false : { scale: 1.9, opacity: 0.2 }}
          whileInView={{ scale: 1, opacity: 1 }}
          viewport={{ once: true, amount: 0.35 }}
          transition={{ duration: 1.8, ease }}
        >
          <Swarm count={100} tall dark />
        </motion.div>
        <p className="fin-cap">One queue, one hundred agents. Touch any agent to see its lease and footprint.</p>
        <div className="know">
          <h2>Now you know how Maidan works.</h2>
          <p>An open-source coordination layer for agent swarms. Speak MCP, A2A, REST, or Python, and host it yourself.</p>
          <div className="codes">
            <figure className="code">
              <figcaption>maidan.yaml <Copy text={y} label="maidan.yaml" /></figcaption>
              <pre><code>{y}</code></pre>
            </figure>
            <figure className="code">
              <figcaption>agent.py <Copy text={PY} label="Python snippet" /></figcaption>
              <pre><code>{PY}</code></pre>
            </figure>
          </div>
          <div className="cmd-row">
            <code className="cmd">$ docker compose up</code>
            <Copy text="docker compose up" label="docker compose command" />
          </div>
          <div className="ctas">
            <a className="cta star" href={GITHUB}>
              <span aria-hidden="true">★</span> Star on GitHub
            </a>
            <button type="button" className="cta ghost" onClick={onJoin}>
              Join the hosted waitlist
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
