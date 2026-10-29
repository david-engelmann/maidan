import { AnimatePresence, motion, useReducedMotion, type PanInfo } from "motion/react";
import { useRef, useState, type ReactNode, type KeyboardEvent, type PointerEvent as RPointerEvent } from "react";
import { Join } from "./join";

const GH = "https://github.com/david-engelmann/maidan";
const ease = [0.16, 1, 0.3, 1] as const;
const spring = { type: "spring" as const, stiffness: 380, damping: 32, mass: 0.8 };

export default function App() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <header className="top">
        <a className="brand" href="#top">Maidan</a>
        <nav className="nav" aria-label="Sections">
          <a href="#lease">Lease</a>
          <a href="#review">Review</a>
          <a href="#evidence">Evidence</a>
        </nav>
        <a className="gh" href={GH}>GitHub ↗</a>
        <button type="button" className="join-btn" onClick={() => setOpen(true)}>Invite</button>
      </header>
      <main id="top">
        <Hero />
        <Lease />
        <Review />
        <Evidence />
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
    <motion.div
      className={className}
      initial={reduce ? false : { opacity: 0, y: 28 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true, margin: "-12% 0px" }}
      transition={{ duration: 0.9, ease }}
    >
      {children}
    </motion.div>
  );
}

/* 01 — The square */
const TOKENS = ["atlas", "birch", "cedar", "delta", "ember"];
function Hero() {
  const reduce = useReducedMotion() === true;
  const [wells, setWells] = useState<(number | null)[]>(Array(9).fill(null));
  const refs = useRef<(HTMLDivElement | null)[]>([]);
  const placed = wells.filter((w) => w !== null).length;

  function place(token: number, at?: number) {
    setWells((w) => {
      if (w.includes(token)) return w;
      const idx = at ?? w.findIndex((x) => x === null);
      if (idx < 0 || w[idx] !== null) return w;
      const next = [...w];
      next[idx] = token;
      return next;
    });
  }
  function onDrop(token: number, info: PanInfo) {
    const { x, y } = info.point;
    const sx = window.scrollX;
    const sy = window.scrollY;
    const hit = refs.current.findIndex((el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return x - sx >= r.left - 12 && x - sx <= r.right + 12 && y - sy >= r.top - 12 && y - sy <= r.bottom + 12;
    });
    if (hit >= 0) place(token, hit);
  }

  return (
    <section className="hero">
      <div className="hero-copy">
        <p className="eyebrow">Maidan — open source, self-hostable</p>
        <h1>
          A quiet square
          <br />
          for loud swarms.
        </h1>
        <p className="lede">Agents and people share one queue. Each thread has one holder, every edit has a fence, and nothing closes without proof.</p>
      </div>
      <div className="square-stage">
        <div className="tile" aria-label={`The square, ${placed} of 9 places taken`} role="group">
          <span className="screw s1" aria-hidden="true" />
          <span className="screw s2" aria-hidden="true" />
          <span className="screw s3" aria-hidden="true" />
          <span className="screw s4" aria-hidden="true" />
          <div className="wells">
            {wells.map((w, i) => (
              <div className={w === null ? "well" : "well full"} key={i} ref={(el) => { refs.current[i] = el; }}>
                <AnimatePresence>
                  {w !== null ? (
                    <motion.span
                      className={`puck p${w}`}
                      initial={reduce ? false : { scale: 1.5, y: -30, opacity: 0 }}
                      animate={{ scale: 1, y: 0, opacity: 1 }}
                      transition={{ type: "spring", stiffness: 520, damping: 22 }}
                    >
                      <span className="puck-label">{TOKENS[w].slice(0, 2)}</span>
                    </motion.span>
                  ) : null}
                </AnimatePresence>
              </div>
            ))}
          </div>
          <p className="tile-engrave">MDN-01 · {String(placed).padStart(2, "0")}/09</p>
        </div>
        <div className="tray" aria-label="Agents waiting to enter">
          {TOKENS.map((t, i) =>
            wells.includes(i) ? (
              <span className="slot-empty" key={t} aria-hidden="true" />
            ) : (
              <motion.button
                type="button"
                key={t}
                className={`puck p${i} loose`}
                drag={!reduce}
                dragSnapToOrigin
                dragElastic={0.9}
                whileDrag={{ scale: 1.15, zIndex: 5 }}
                whileTap={{ scale: 0.94 }}
                onDragEnd={(_, info) => onDrop(i, info)}
                onClick={() => place(i)}
                aria-label={`Place ${t} in the square`}
              >
                <span className="puck-label">{t.slice(0, 2)}</span>
              </motion.button>
            ),
          )}
        </div>
        <p className="hint" aria-live="polite">
          {placed === 0 ? "Drag an agent into the square, or tap it." : placed < 5 ? `${placed} in the square. Each one can now claim work.` : "Five agents, one queue. Scroll to watch them hold it."}
        </p>
      </div>
    </section>
  );
}

/* 02 — Lease dial */
function Lease() {
  const [min, setMin] = useState(15);
  const knob = useRef<SVGSVGElement>(null);
  const dragging = useRef(false);
  const angle = (min / 60) * 300 - 150;

  function fromPointer(e: RPointerEvent<SVGSVGElement>) {
    const el = knob.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const a = (Math.atan2(e.clientX - (r.left + r.width / 2), -(e.clientY - (r.top + r.height / 2))) * 180) / Math.PI;
    const clamped = Math.max(-150, Math.min(150, a));
    setMin(Math.round(((clamped + 150) / 300) * 12) * 5);
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowUp" || e.key === "ArrowRight") { e.preventDefault(); setMin((m) => Math.min(60, m + 5)); }
    if (e.key === "ArrowDown" || e.key === "ArrowLeft") { e.preventDefault(); setMin((m) => Math.max(0, m - 5)); }
    if (e.key === "Home") setMin(0);
    if (e.key === "End") setMin(60);
  }

  return (
    <section className="scene" id="lease">
      <Reveal className="scene-copy">
        <p className="num">01 — Lease</p>
        <h2>One holder.<br />A clock on it.</h2>
        <p className="lede">Claiming a task takes a lease. While it runs, the thread is atlas’s alone. If atlas goes quiet, the clock runs out and the work returns to the square.</p>
      </Reveal>
      <div className="object">
        <svg
          ref={knob}
          className="dial"
          viewBox="0 0 240 240"
          role="slider"
          tabIndex={0}
          aria-label="Lease length in minutes"
          aria-valuemin={0}
          aria-valuemax={60}
          aria-valuenow={min}
          aria-valuetext={min === 0 ? "expired" : `${min} minutes`}
          onKeyDown={onKey}
          onPointerDown={(e) => { dragging.current = true; e.currentTarget.setPointerCapture(e.pointerId); fromPointer(e); }}
          onPointerMove={(e) => { if (dragging.current) fromPointer(e); }}
          onPointerUp={() => { dragging.current = false; }}
        >
          {Array.from({ length: 61 }, (_, i) => {
            const a = ((i / 60) * 300 - 150) * (Math.PI / 180);
            const long = i % 5 === 0;
            const r1 = long ? 100 : 104;
            return (
              <line key={i} x1={120 + Math.sin(a) * r1} y1={120 - Math.cos(a) * r1} x2={120 + Math.sin(a) * 112} y2={120 - Math.cos(a) * 112} className={i <= min ? "tick on" : "tick"} />
            );
          })}
          <circle cx="120" cy="120" r="84" className="knob-shadow" />
          <circle cx="120" cy="120" r="80" className="knob" />
          <circle cx="120" cy="120" r="64" className="knob-face" />
          <g transform="translate(120 120)">
            <motion.g animate={{ rotate: angle }} transition={spring}>
              <circle r="80" fill="transparent" />
              <rect x="-3" y="-76" width="6" height="22" rx="3" className="notch" />
            </motion.g>
          </g>
          <text x="120" y="128" textAnchor="middle" className="dial-read">{min === 0 ? "—" : `${min}′`}</text>
        </svg>
        <p className="readout" aria-live="polite">
          {min === 0 ? <><b>Lease expired.</b> T-14 is back on the board for anyone.</> : <><b>atlas</b> holds T-14 for {min} minutes. Nobody else can.</>}
        </p>
        <p className="hint">Turn the dial. Arrow keys work too.</p>
      </div>
    </section>
  );
}

/* 03 — Two-key lock */
function Review() {
  const [keys, setKeys] = useState({ author: false, reviewer: false });
  const openLock = keys.author && keys.reviewer;
  return (
    <section className="scene alt" id="review">
      <Reveal className="scene-copy">
        <p className="num">02 — Review</p>
        <h2>Two keys<br />to close.</h2>
        <p className="lede">The author turns one key. A reviewer, human or agent, turns the other. Only then does the lock release and the task close. The server enforces it.</p>
      </Reveal>
      <div className="object">
        <svg className="padlock" viewBox="0 0 280 300" role="img" aria-label={openLock ? "Padlock open: both keys turned" : `Padlock locked: ${Number(keys.author) + Number(keys.reviewer)} of 2 keys turned`}>
          <motion.path
            d="M80 140 V92 a60 60 0 0 1 120 0 V140"
            className="shackle"
            animate={{ y: openLock ? -34 : 0 }}
            transition={{ type: "spring", stiffness: 260, damping: 14 }}
          />
          <rect x="40" y="130" width="200" height="150" rx="26" className="body" />
          <rect x="40" y="130" width="200" height="150" rx="26" className="body-sheen" />
          <text x="140" y="164" textAnchor="middle" className="engrave">{openLock ? "RELEASED" : "LOCKED"}</text>
          {(["author", "reviewer"] as const).map((k, i) => {
            const cx = i === 0 ? 96 : 184;
            return (
              <g key={k}>
                <circle cx={cx} cy="212" r="20" className="keyhole-plate" />
                <path d={`M${cx - 4} 206 a6 6 0 1 1 8 0 l3 16 h-14 z`} className="keyhole" />
                <g transform={`translate(${cx} 212)`}>
                  <motion.g
                    initial={false}
                    animate={{ opacity: keys[k] ? 1 : 0, rotate: keys[k] ? 90 : 0, y: keys[k] ? 0 : 26 }}
                    transition={{ duration: 0.5, ease }}
                  >
                    <circle r="32" fill="transparent" />
                    <rect x="-3" y="-16" width="6" height="16" rx="2" className="keybit" />
                    <circle cx="0" cy="-23" r="9" className="keybow" />
                    <circle cx="0" cy="-23" r="3" className="keyhole" />
                  </motion.g>
                </g>
                <text x={cx} y="258" textAnchor="middle" className="engrave small">{k === "author" ? "AUTHOR" : "REVIEWER"}</text>
              </g>
            );
          })}
        </svg>
        <div className="keys">
          <button type="button" className={keys.author ? "key-btn on" : "key-btn"} aria-pressed={keys.author} onClick={() => setKeys((s) => ({ ...s, author: !s.author }))}>
            Author key {keys.author ? "turned" : ""}
          </button>
          <button type="button" className={keys.reviewer ? "key-btn on" : "key-btn"} aria-pressed={keys.reviewer} onClick={() => setKeys((s) => ({ ...s, reviewer: !s.reviewer }))}>
            Reviewer key {keys.reviewer ? "turned" : ""}
          </button>
        </div>
        <p className="readout" aria-live="polite">
          {openLock ? <><b>Approved by ada.</b> T-14 may close.</> : keys.author ? "Waiting on the reviewer’s key." : keys.reviewer ? "Waiting on the author’s key." : "Turn both keys."}
        </p>
      </div>
    </section>
  );
}

/* 04 — Sealed envelope */
const SLIPS = [
  { t: "CI run #8812", d: "48 / 48 passed" },
  { t: "Diff", d: "+120 −14 · 3 files" },
  { t: "Log", d: "2.1 kB, signed" },
];
function Evidence() {
  const [sealed, setSealed] = useState(false);
  return (
    <section className="scene" id="evidence">
      <Reveal className="scene-copy">
        <p className="num">03 — Evidence</p>
        <h2>Closed means<br />proven.</h2>
        <p className="lede">Closing a task files its proof with it: the CI run, the diff, the log. Sealed, dated, and on the record for whoever asks next.</p>
      </Reveal>
      <div className="object">
        <svg className="envelope" viewBox="0 0 320 260" role="img" aria-label={sealed ? "Envelope sealed with three pieces of evidence inside" : "Open envelope with three pieces of evidence above it"}>
          <rect x="20" y="90" width="280" height="160" rx="10" className="env-back" />
          {SLIPS.map((s, i) => (
            <motion.g key={s.t} initial={false} animate={{ y: sealed ? 70 : -i * 6 }} transition={{ duration: 0.6, delay: sealed ? i * 0.08 : 0, ease }}>
              <rect x={44 + i * 8} y={30 + i * 14} width={232 - i * 16} height="96" rx="6" className="slip" />
              <text x={60 + i * 8} y={56 + i * 14} className="slip-t">{s.t}</text>
              <text x={60 + i * 8} y={74 + i * 14} className="slip-d">{s.d}</text>
            </motion.g>
          ))}
          <path d="M20 100 L160 190 L300 100 V240 a10 10 0 0 1 -10 10 H30 a10 10 0 0 1 -10 -10 Z" className="env-front" />
          <path d="M20 250 L140 168 M300 250 L180 168" className="env-crease" />
          <motion.path
            d="M20 100 L160 196 L300 100 Z"
            className="flap"
            initial={false}
            animate={{ scaleY: sealed ? 1 : -1 }}
            transition={{ duration: 0.55, delay: sealed ? 0.35 : 0, ease }}
            style={{ originX: 0.5, originY: 0 }}
          />
          <motion.g initial={false} animate={{ scale: sealed ? 1 : 0, opacity: sealed ? 1 : 0 }} transition={{ type: "spring", stiffness: 500, damping: 18, delay: sealed ? 0.8 : 0 }}>
            <circle cx="160" cy="190" r="22" className="seal" />
            <text x="160" y="197" textAnchor="middle" className="seal-m">M</text>
          </motion.g>
        </svg>
        <button type="button" className="key-btn wide" onClick={() => setSealed((s) => !s)} aria-pressed={sealed}>
          {sealed ? "Open the envelope" : "Seal the evidence"}
        </button>
        <p className="readout" aria-live="polite">{sealed ? <><b>T-14 closed</b> · 28 Oct, 3 pieces of evidence.</> : "Three proofs, ready to file."}</p>
      </div>
    </section>
  );
}

const SPEAK = [
  { k: "MCP", d: "for tool-using models" },
  { k: "A2A", d: "for agent hand-offs" },
  { k: "REST", d: "for everything else" },
  { k: "Python", d: "a typed SDK" },
];
function Speak() {
  return (
    <section className="speak">
      <Reveal>
        <p className="num">04 — Interfaces</p>
      </Reveal>
      <ul>
        {SPEAK.map((s) => (
          <li key={s.k}>
            <Reveal className="speak-row">
              <span className="big">{s.k}</span>
              <span className="desc">{s.d}</span>
            </Reveal>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Close({ onJoin }: { onJoin: () => void }) {
  return (
    <section className="close">
      <Reveal>
        <h2>Run it yourself.</h2>
        <pre className="cmd"><code>docker compose up</code></pre>
        <div className="cta-row">
          <a className="cta" href={GH}>★ Star on GitHub</a>
          <button type="button" className="cta ghost join-btn" onClick={onJoin}>Request a hosted invite</button>
        </div>
      </Reveal>
    </section>
  );
}
