import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState, type PointerEvent as RPointerEvent, type ReactNode } from "react";
import { Join } from "./join";
import {
  LEASE_SECONDS,
  MAIN,
  QUEUE,
  REVIEW,
  addAgent,
  agents,
  initial,
  moveNode,
  pulsePoint,
  release,
  setTarget,
  step,
  type Camera,
  type Node,
  type Scene,
  type Sim,
} from "./sim";

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
      <header className="top">
        <a className="brand" href="#top">
          <span className="brand-dot" aria-hidden="true" />
          Maidan
        </a>
        <span className="top-space" />
        <a className="top-link" href={GITHUB}>
          ★ GitHub
        </a>
        <button type="button" className="top-join" onClick={join}>
          Join
        </button>
      </header>
      <main id="top">
        <Story onJoin={join} />
        <Finale onJoin={join} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function usePortrait() {
  const [portrait, setPortrait] = useState(() => window.innerWidth < 900);
  useEffect(() => {
    const on = () => setPortrait(window.innerWidth < 900);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return portrait;
}

function focusFor(scene: Scene, s: Sim): [number, number, number, number] {
  // [x, y, zoom portrait, zoom landscape]
  const first = agents(s)[0];
  if (scene === "lease" && first) return [first.x, first.y, 2.5, 2.3];
  if (scene === "gate") return [0, 250, 2.3, 2.1];
  if (scene === "evidence") return [0, 330, 2.3, 2.1];
  if (scene === "swarm") return [0, 30, 0.6, 0.82];
  return [0, 97, 1, 1];
}

function targetCam(scene: Scene, s: Sim, portrait: boolean): Camera {
  const [x, y, zp, zl] = focusFor(scene, s);
  if (portrait) {
    if (scene === "home") return { cx: 0, cy: -244, s: 0.72 };
    return { cx: x, cy: y + 0.16 * (1000 / zp), s: zp };
  }
  const z = zl;
  return { cx: x - 0.2 * (1000 / z), cy: y, s: z };
}

function Story({ onJoin }: { onJoin: () => void }) {
  const portrait = usePortrait();
  const reduce = useReducedMotion() ?? false;
  const [sim, setSim] = useState<Sim>(() => {
    const s = initial();
    return { ...s, cam: targetCam("home", s, window.innerWidth < 900) };
  });
  const [scene, setScene] = useState<Scene>("home");
  const [selected, setSelected] = useState<number | null>(null);
  const [running, setRunning] = useState(true);
  const live = useRef({ scene, portrait, reduce });
  const storyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    live.current = { scene, portrait, reduce };
  }, [scene, portrait, reduce]);

  useEffect(() => {
    const el = storyRef.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setRunning(e.isIntersecting && !document.hidden));
    io.observe(el);
    return () => io.disconnect();
  }, []);

  useEffect(() => {
    if (!running) return;
    let raf = 0;
    let last = performance.now();
    const frame = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const { scene: sc, portrait: p, reduce: r } = live.current;
      setSim((s) => step(s, dt, targetCam(sc, s, p), r ? 0.4 : 1, sc === "lease" ? (agents(s)[0]?.id ?? null) : null));
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [running]);

  useEffect(() => {
    const steps = Array.from(document.querySelectorAll<HTMLElement>("[data-scene]"));
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) {
            const next = e.target.getAttribute("data-scene") as Scene;
            setScene(next);
            setSim((s) => setTarget(s, next));
          }
      },
      { rootMargin: "-45% 0px -45% 0px" },
    );
    steps.forEach((s) => io.observe(s));
    return () => io.disconnect();
  }, []);

  const sel = sim.nodes.find((n) => n.id === selected) ?? null;
  const list = agents(sim);
  const holding = list.filter((n) => n.task).length;

  return (
    <div className="story" ref={storyRef}>
      <div className="stage">
        <Graph
          sim={sim}
          scene={scene}
          selected={selected}
          onSelect={setSelected}
          onMove={(id, x, y) => setSim((s) => moveNode(s, id, x, y))}
          onRelease={() => setSim(release)}
          onAdd={(x, y) => setSim((s) => addAgent(s, x, y))}
        />
        <div className="hud" aria-live="polite">
          <span>
            <b>{list.length}</b> agents
          </span>
          <span>
            <b>{holding}</b> holding
          </span>
          <span>
            <b>{sim.shipped}</b> shipped
          </span>
          <span className="hud-ok">
            <b>0</b> collisions
          </span>
        </div>
        <AnimatePresence>{sel ? <Inspect key={sel.id} node={sel} sim={sim} onClose={() => setSelected(null)} /> : null}</AnimatePresence>
      </div>

      <div className="steps">
        <section className="step hero" data-scene="home">
          <div className="card">
            <p className="eyebrow">Open-source coordination for AI agent swarms</p>
            <h1>The orchestra.</h1>
            <p className="lede">
              Every dot is an agent. Every light is a task. <span className="nb">Nobody collides,</span> because Maidan keeps the score.
            </p>
            <p className="hint">Drag a node · tap to inspect · hold empty space to add an agent</p>
            <div className="row">
              <button type="button" className="cta" onClick={() => setSim((s) => addAgent(s))}>
                + Add an agent
              </button>
              <a className="cta ghost-cta" href={GITHUB}>
                ★ Star on GitHub
              </a>
            </div>
          </div>
        </section>
        <Step scene="lease" n="1" title="A task is a lease." color="lease">
          When <b className="c-claim">Ada</b> claims a task she holds it alone, for fifteen seconds at a time. Her heartbeat renews the{" "}
          <b className="c-lease">amber ring</b>. If she goes quiet the lease lapses and the task goes back to the queue for someone else.
        </Step>
        <Step scene="gate" n="2" title="Some work waits for a human." color="review">
          Finished work flows into the <b className="c-review">review gate</b>. It only moves on once it has the approvals it needs. Agents can't
          wave their own work through.
        </Step>
        <Step scene="evidence" n="3" title="Nothing closes without proof." color="proof">
          To reach <b className="c-proof">main</b>, a task carries its evidence: the test log, the diff, the pull request. No proof, no close,
          and the record stays for whoever asks later.
        </Step>
        <Step scene="swarm" n="4" title="Now make it a hundred." color="claim">
          Same three rules, a hundred agents. Footprints keep two agents off the same file, so the collision counter stays at{" "}
          <b className="c-proof">zero</b>.
          <span className="big">
            <b>{sim.claims}</b> claims · <b>{sim.shipped}</b> shipped · <b>0</b> collisions
          </span>
          <button type="button" className="cta inline" onClick={onJoin}>
            Join the waitlist
          </button>
        </Step>
      </div>
    </div>
  );
}

function Step({ scene, n, title, color, children }: { scene: Scene; n: string; title: string; color: string; children: ReactNode }) {
  return (
    <section className="step" data-scene={scene}>
      <motion.div
        className="card"
        initial={{ opacity: 0, y: 24 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ amount: 0.6 }}
        transition={{ type: "spring", stiffness: 260, damping: 30 }}
      >
        <span className={`num bg-${color}`}>{n}</span>
        <h2>{title}</h2>
        <p>{children}</p>
      </motion.div>
    </section>
  );
}

interface GraphProps {
  sim: Sim;
  scene: Scene;
  selected: number | null;
  onSelect: (id: number | null) => void;
  onMove: (id: number, x: number, y: number) => void;
  onRelease: () => void;
  onAdd: (x: number, y: number) => void;
}

function Graph({ sim, scene, selected, onSelect, onMove, onRelease, onAdd }: GraphProps) {
  const camRef = useRef<SVGGElement>(null);
  const press = useRef<{ id: number | null; x: number; y: number; moved: boolean; timer: number }>({ id: null, x: 0, y: 0, moved: false, timer: 0 });
  const [ripple, setRipple] = useState<{ x: number; y: number; k: number } | null>(null);
  const { cam } = sim;
  const list = agents(sim);
  const crowd = list.length > 12;

  const world = (cx: number, cy: number): [number, number] => {
    const g = camRef.current;
    const m = g?.getScreenCTM();
    if (!m) return [0, 0];
    const p = new DOMPoint(cx, cy).matrixTransform(m.inverse());
    return [p.x, p.y];
  };

  const down = (e: RPointerEvent, id: number | null) => {
    e.stopPropagation();
    const p = press.current;
    window.clearTimeout(p.timer);
    p.id = id;
    p.x = e.clientX;
    p.y = e.clientY;
    p.moved = false;
    if (id !== null) (e.currentTarget as Element).setPointerCapture(e.pointerId);
    if (id === null) {
      const [wx, wy] = world(e.clientX, e.clientY);
      p.timer = window.setTimeout(() => {
        if (!press.current.moved) {
          onAdd(wx, wy);
          setRipple({ x: wx, y: wy, k: Date.now() });
        }
      }, 520);
    }
  };
  const move = (e: RPointerEvent) => {
    const p = press.current;
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 7) {
      p.moved = true;
      window.clearTimeout(p.timer);
    }
    if (p.id !== null && p.moved && p.id !== QUEUE) {
      const [wx, wy] = world(e.clientX, e.clientY);
      onMove(p.id, wx, wy);
    }
  };
  const up = () => {
    const p = press.current;
    window.clearTimeout(p.timer);
    if (p.id !== null) {
      if (!p.moved) onSelect(p.id === selected ? null : p.id);
      else onRelease();
    } else if (!p.moved) onSelect(null);
    p.id = null;
  };

  const queue = sim.nodes[QUEUE];
  const review = sim.nodes[REVIEW];
  const main = sim.nodes[MAIN];
  const first = list[0];

  return (
    <svg
      className="graph"
      viewBox="-500 -500 1000 1000"
      preserveAspectRatio="xMidYMid slice"
      onPointerDown={(e) => down(e, null)}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={() => {
        window.clearTimeout(press.current.timer);
        press.current.id = null;
      }}
      role="img"
      aria-label={`Graph of ${list.length} agents claiming tasks from a queue, passing a review gate and shipping to main.`}
    >
      <defs>
        <radialGradient id="glow">
          <stop offset="0" stopColor="#fff" stopOpacity="0.9" />
          <stop offset="1" stopColor="#fff" stopOpacity="0" />
        </radialGradient>
      </defs>
      <g ref={camRef} transform={`scale(${cam.s}) translate(${-cam.cx} ${-cam.cy})`}>
        <g className="edges">
          {list.map((n) => (
            <g key={n.id}>
              <line x1={queue.x} y1={queue.y} x2={n.x} y2={n.y} className={n.task ? "edge hot" : "edge"} />
              {!crowd ? <line x1={n.x} y1={n.y} x2={review.x} y2={review.y} className="edge faint" /> : null}
            </g>
          ))}
          <line x1={review.x} y1={review.y} x2={main.x} y2={main.y} className="edge gate" />
        </g>

        {sim.pulses.map((p) => {
          if (p.t < 0) return null;
          const [x, y] = pulsePoint(sim, p);
          return (
            <g key={p.id} className={`pulse p-${p.kind}`} transform={`translate(${x} ${y})`}>
              <circle r={crowd ? 9 : 12} className="pulse-halo" />
              <circle r={crowd ? 3 : 4.5} className="pulse-core" />
            </g>
          );
        })}

        <g transform={`translate(${queue.x} ${queue.y})`} className="node-fixed">
          <rect x={-34} y={-34} width={68} height={68} rx={18} className="queue" />
          <text y={5} className="label strong">
            Queue
          </text>
        </g>
        <g
          transform={`translate(${review.x} ${review.y})`}
          className={`node-fixed tap ${selected === REVIEW ? "is-sel" : ""}`}
          onPointerDown={(e) => down(e, REVIEW)}
        >
          <rect x={-26} y={-26} width={52} height={52} rx={10} transform="rotate(45)" className="review" />
          <text y={4} className="label strong">
            Review
          </text>
        </g>
        <g transform={`translate(${main.x} ${main.y})`} className="node-fixed tap" onPointerDown={(e) => down(e, MAIN)}>
          <circle r={30} className="main" />
          <text y={-2} className="label strong">
            main
          </text>
          <text y={12} className="label tiny">
            {sim.shipped} shipped
          </text>
        </g>

        {list.map((n, i) => (
          <Agent key={n.id} node={n} small={crowd && i >= 6} selected={selected === n.id} onDown={(e) => down(e, n.id)} />
        ))}

        {scene === "lease" && first ? (
          <g transform={`translate(${first.x} ${first.y + 40})`} className="note">
            <text className="note-text c-lease-fill">
              {first.task ? `lease ${(first.lease * LEASE_SECONDS).toFixed(1)}s · renews on heartbeat` : "claiming next task…"}
            </text>
          </g>
        ) : null}
        {scene === "gate" ? (
          <g transform={`translate(${review.x - 44} ${review.y})`} className="note">
            <text className="note-text end c-review-fill">
              <tspan x={0} dy={-3}>1 approval</tspan>
              <tspan x={0} dy={12}>
                {sim.reviewed} passed
              </tspan>
            </text>
          </g>
        ) : null}
        {scene === "evidence" ? (
          <g transform={`translate(${main.x} ${main.y + 52})`} className="note">
            <text className="note-text c-proof-fill">test log ✓ · diff ✓ · PR ✓</text>
          </g>
        ) : null}

        <AnimatePresence>
          {ripple ? (
            <motion.circle
              key={ripple.k}
              cx={ripple.x}
              cy={ripple.y}
              className="ripple"
              initial={{ r: 4, opacity: 0.9 }}
              animate={{ r: 46, opacity: 0 }}
              transition={{ duration: 0.6 }}
              onAnimationComplete={() => setRipple(null)}
            />
          ) : null}
        </AnimatePresence>
      </g>
    </svg>
  );
}

function arc(r: number, f: number): string {
  const c = 2 * Math.PI * r;
  return `${c * Math.max(0, Math.min(1, f))} ${c}`;
}

function Agent({ node, small, selected, onDown }: { node: Node; small: boolean; selected: boolean; onDown: (e: RPointerEvent) => void }) {
  const r = small ? 9 : 19;
  return (
    <g
      transform={`translate(${node.x} ${node.y})`}
      className={`agent ${node.phase} ${selected ? "is-sel" : ""}`}
      onPointerDown={onDown}
    >
      <circle r={r + 10} className="hit" />
      <circle r={r} className="agent-body" />
      {node.phase === "working" ? (
        <>
          <circle r={r + 4} className="ring-lease" strokeDasharray={arc(r + 4, node.lease)} transform="rotate(-90)" />
          <circle r={r - 4} className="ring-work" strokeDasharray={arc(r - 4, node.progress)} transform="rotate(-90)" />
        </>
      ) : null}
      {!small ? (
        <text y={4} className="label">
          {node.name}
        </text>
      ) : null}
    </g>
  );
}

function Inspect({ node, sim, onClose }: { node: Node; sim: Sim; onClose: () => void }) {
  let rows: [string, string][];
  if (node.kind === "agent") {
    rows = [
      ["State", node.phase === "working" ? "working" : node.phase === "claiming" ? "claiming" : "idle, looking for work"],
      ["Holds", node.task ? node.task.name : "nothing"],
      ["Lease", node.phase === "working" ? `${(node.lease * LEASE_SECONDS).toFixed(1)}s left, renews every 3s` : "none"],
      ["Footprint", node.task ? node.task.files.join(", ") : "none"],
    ];
  } else if (node.kind === "review") {
    rows = [
      ["Rule", "1 approval before merge"],
      ["Passed", String(sim.reviewed)],
      ["Waiting", String(sim.pulses.filter((p) => p.kind === "review").length)],
    ];
  } else {
    rows = [
      ["Shipped", String(sim.shipped)],
      ["Evidence", "test log, diff, pull request"],
      ["Collisions", "0"],
    ];
  }
  return (
    <motion.aside
      className="inspect"
      initial={{ opacity: 0, y: 16, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 12, scale: 0.97 }}
      transition={{ type: "spring", stiffness: 420, damping: 32 }}
      aria-label={`Inspect ${node.name}`}
    >
      <div className="inspect-head">
        <b>{node.name}</b>
        <button type="button" className="x" onClick={onClose} aria-label="Close">
          ×
        </button>
      </div>
      <dl>
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
    </motion.aside>
  );
}

function Finale({ onJoin }: { onJoin: () => void }) {
  return (
    <section className="finale">
      <h2>Your agents, in concert.</h2>
      <p>
        Maidan is an open-source server that hands out leased claims, enforces footprints and review gates, and keeps the evidence. Plug in any
        agent that speaks a standard protocol.
      </p>
      <ul className="protos">
        {["MCP", "A2A", "REST", "Python SDK"].map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
      <pre className="cmd">
        <code>docker compose up</code>
      </pre>
      <div className="row">
        <button type="button" className="cta" onClick={onJoin}>
          Join the waitlist
        </button>
        <a className="cta ghost-cta" href={GITHUB}>
          ★ Star on GitHub
        </a>
      </div>
      <p className="foot">© Maidan · Open source</p>
    </section>
  );
}

