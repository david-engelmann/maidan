import { motion } from "motion/react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { FILM, fitCanvas, runWhileVisible } from "./loop";
import { LEASE_SECONDS, MAIN, QUEUE, REVIEW, addAgent, agents, initial, moveNode, pulsePoint, release, setTarget, step, type Camera, type Scene, type Sim } from "./sim";

function focusFor(scene: Scene, s: Sim): [number, number, number, number] {
  const first = agents(s)[0];
  if (scene === "lease" && first) return [first.x, first.y, 2.5, 2.4];
  if (scene === "gate") return [0, 250, 2.3, 2.2];
  if (scene === "evidence") return [0, 330, 2.3, 2.2];
  if (scene === "swarm") return [0, 40, 0.62, 0.86];
  return [0, 120, 1, 1.12];
}

function targetCam(scene: Scene, s: Sim, portrait: boolean): Camera {
  const [x, y, zp, zl] = focusFor(scene, s);
  if (portrait) {
    if (scene === "home") return { cx: 0, cy: 40, s: 0.86 };
    return { cx: x, cy: y + 0.17 * (1000 / zp), s: zp };
  }
  return { cx: x - 0.2 * (1000 / zl), cy: y, s: zl };
}

const C = { claim: "45,212,191", lease: "251,191,36", review: "167,139,250", proof: "74,222,128", ink: "242,243,247" };

function draw(ctx: CanvasRenderingContext2D, s: Sim, scene: Scene, w: number, h: number, dpr: number, selected: number | null, t: number) {
  const k = Math.max(w, h) / 1000;
  const z = s.cam.s * k;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.setTransform(dpr * z, 0, 0, dpr * z, dpr * (w / 2 - s.cam.cx * z), dpr * (h / 2 - s.cam.cy * z));
  const list = agents(s);
  const crowd = list.length > 12;
  const q = s.nodes[QUEUE];
  const rv = s.nodes[REVIEW];
  const mn = s.nodes[MAIN];
  const px = 1 / z;
  // Faint orbit rings fill the stage.
  ctx.lineWidth = px;
  for (let r = 150; r <= 600; r += 90) {
    ctx.strokeStyle = `rgba(255,255,255,${0.05 - r / 20000})`;
    ctx.beginPath();
    ctx.arc(0, 0, r, 0, Math.PI * 2);
    ctx.stroke();
  }
  for (const n of list) {
    ctx.strokeStyle = n.task ? `rgba(${C.claim},0.34)` : "rgba(255,255,255,0.08)";
    ctx.lineWidth = n.task ? 1.4 : 1;
    ctx.beginPath();
    ctx.moveTo(q.x, q.y);
    ctx.lineTo(n.x, n.y);
    ctx.stroke();
    if (!crowd) {
      ctx.setLineDash([3, 5]);
      ctx.strokeStyle = `rgba(${C.review},0.1)`;
      ctx.beginPath();
      ctx.moveTo(n.x, n.y);
      ctx.lineTo(rv.x, rv.y);
      ctx.stroke();
      ctx.setLineDash([]);
    }
  }
  ctx.setLineDash([4, 6]);
  ctx.lineDashOffset = -t * 20;
  ctx.strokeStyle = `rgba(${C.proof},0.45)`;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(rv.x, rv.y);
  ctx.lineTo(mn.x, mn.y);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.lineDashOffset = 0;

  for (const p of s.pulses) {
    if (p.t < 0) continue;
    const [x, y] = pulsePoint(s, p);
    const col = p.kind === "claim" ? C.claim : p.kind === "review" ? C.review : C.proof;
    const r = crowd ? 9 : 13;
    const g = ctx.createRadialGradient(x, y, 0, x, y, r);
    g.addColorStop(0, `rgba(${col},0.6)`);
    g.addColorStop(1, `rgba(${col},0)`);
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = `rgb(${col})`;
    ctx.beginPath();
    ctx.arc(x, y, crowd ? 2.6 : 4, 0, Math.PI * 2);
    ctx.fill();
  }

  const label = (text: string, x: number, y: number, size: number, col: string, weight = 700, align: CanvasTextAlign = "center") => {
    ctx.font = `${weight} ${size}px Geist, system-ui, sans-serif`;
    ctx.textAlign = align;
    ctx.textBaseline = "middle";
    ctx.lineWidth = 3;
    ctx.strokeStyle = "#06070b";
    ctx.strokeText(text, x, y);
    ctx.fillStyle = col;
    ctx.fillText(text, x, y);
  };

  // Queue.
  ctx.beginPath();
  ctx.roundRect(q.x - 34, q.y - 34, 68, 68, 18);
  ctx.fillStyle = `rgba(${C.claim},0.12)`;
  ctx.fill();
  ctx.strokeStyle = `rgb(${C.claim})`;
  ctx.lineWidth = 2;
  ctx.stroke();
  label("Queue", q.x, q.y, 12.5, `rgb(${C.ink})`, 750);
  // Review diamond.
  ctx.save();
  ctx.translate(rv.x, rv.y);
  ctx.rotate(Math.PI / 4);
  ctx.beginPath();
  ctx.roundRect(-26, -26, 52, 52, 10);
  ctx.fillStyle = `rgba(${C.review},0.16)`;
  ctx.fill();
  ctx.strokeStyle = `rgb(${C.review})`;
  ctx.lineWidth = selected === REVIEW ? 3.5 : 2;
  ctx.stroke();
  ctx.restore();
  label("Review", rv.x, rv.y, 12, `rgb(${C.ink})`, 750);
  // main.
  ctx.beginPath();
  ctx.arc(mn.x, mn.y, 30, 0, Math.PI * 2);
  ctx.fillStyle = `rgba(${C.proof},0.14)`;
  ctx.fill();
  ctx.strokeStyle = `rgb(${C.proof})`;
  ctx.lineWidth = selected === MAIN ? 3.5 : 2;
  ctx.stroke();
  label("main", mn.x, mn.y - 3, 12, `rgb(${C.ink})`, 750);
  label(`${s.shipped} shipped`, mn.x, mn.y + 11, 8.5, "rgba(242,243,247,0.72)", 500);

  list.forEach((n, i) => {
    const small = crowd && i >= 6;
    const r = small ? 9 : 19;
    ctx.beginPath();
    ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
    ctx.fillStyle = "#141824";
    ctx.fill();
    ctx.lineWidth = selected === n.id ? 2.5 : 1.5;
    ctx.strokeStyle = selected === n.id ? "#fff" : n.phase === "working" ? `rgba(${C.claim},0.9)` : n.phase === "claiming" ? `rgba(${C.claim},0.5)` : "rgba(255,255,255,0.28)";
    ctx.stroke();
    if (n.phase === "working") {
      ctx.lineCap = "round";
      ctx.lineWidth = small ? 1.6 : 2.4;
      ctx.strokeStyle = `rgb(${C.lease})`;
      ctx.beginPath();
      ctx.arc(n.x, n.y, r + 4, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * n.lease);
      ctx.stroke();
      ctx.lineWidth = small ? 1.2 : 2;
      ctx.strokeStyle = `rgba(${C.claim},0.85)`;
      ctx.beginPath();
      ctx.arc(n.x, n.y, Math.max(2, r - 4), -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * Math.min(1, n.progress));
      ctx.stroke();
      ctx.lineCap = "butt";
    }
    if (!small) label(n.name, n.x, n.y + 0.5, 11, `rgb(${C.ink})`, 600);
  });

  const first = list[0];
  if (scene === "lease" && first) {
    label(first.task ? `lease ${(first.lease * LEASE_SECONDS).toFixed(1)}s · renews on heartbeat` : "claiming next task…", first.x, first.y - 58, 8, `rgb(${C.lease})`, 650);
    if (first.task) label(first.task.name, first.x, first.y - 34, 8.5, "rgba(242,243,247,0.8)", 600);
  }
  if (scene === "gate") {
    label("1 approval", rv.x - 44, rv.y - 6, 9, `rgb(${C.review})`, 650, "right");
    label(`${s.reviewed} passed`, rv.x - 44, rv.y + 7, 9, `rgb(${C.review})`, 650, "right");
  }
  if (scene === "evidence") label("test log ✓ · diff ✓ · PR ✓", mn.x, mn.y + 52, 9, `rgb(${C.proof})`, 650);
}

type Snap = { node: Sim["nodes"][number]; sim: Pick<Sim, "reviewed" | "shipped" | "pulses"> };
function take(s: Sim, id: number | null): Snap | null {
  const n = id === null ? undefined : s.nodes.find((m) => m.id === id);
  return n ? { node: { ...n }, sim: { reviewed: s.reviewed, shipped: s.shipped, pulses: s.pulses.slice() } } : null;
}

export function Stage({ reduce, onJoin }: { reduce: boolean; onJoin: () => void }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const sim = useRef<Sim>(initial());
  const scene = useRef<Scene>("home");
  const sel = useRef<number | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [snap, setSnap] = useState<Snap | null>(null);
  const hud = useRef<Record<string, HTMLElement | null>>({});
  const [say, setSay] = useState("");

  useEffect(() => {
    const cv = canvas.current;
    const ctx = cv?.getContext("2d");
    if (!cv || !ctx) return;
    let acc = 0;
    let t = 0;
    return runWhileVisible(cv, (dt) => {
      const { w, h, dpr } = fitCanvas(cv);
      const portrait = w < 900;
      const s = sim.current;
      const sc = scene.current;
      sim.current = step(s, dt, targetCam(sc, s, portrait), reduce ? 0.4 : 1, sc === "lease" ? (agents(s)[0]?.id ?? null) : null);
      t += dt;
      draw(ctx, sim.current, sc, w, h, dpr, sel.current, t);
      acc += dt;
      if (acc > 0.25) {
        acc = 0;
        const n = sim.current;
        const list = agents(n);
        const set = (k: string, v: number) => {
          const el = hud.current[k];
          if (el) el.textContent = String(v);
        };
        set("agents", list.length);
        set("holding", list.filter((a) => a.task).length);
        set("shipped", n.shipped);
        set("claims", n.claims);
        set("shipped2", n.shipped);
        if (sel.current !== null) setSnap(take(sim.current, sel.current));
      }
    });
  }, [reduce]);

  useEffect(() => {
    const steps = Array.from(document.querySelectorAll<HTMLElement>("[data-scene]"));
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries)
          if (e.isIntersecting) {
            const next = e.target.getAttribute("data-scene") as Scene;
            scene.current = next;
            sim.current = setTarget(sim.current, next);
            const s = sim.current;
            if (next === "swarm") setSay("Now a hundred agents. Zero collisions.");
            else setSay(`${agents(s).length} agents. ${s.shipped} shipped. Zero collisions.`);
          }
      },
      { rootMargin: "-45% 0px -45% 0px" },
    );
    steps.forEach((s) => io.observe(s));
    return () => io.disconnect();
  }, []);

  // Tap to inspect, drag with a mouse. Touch keeps scrolling the page.
  const press = useRef<{ id: number | null; x: number; y: number; moved: boolean; touch: boolean }>({ id: null, x: 0, y: 0, moved: false, touch: false });
  const world = (cx: number, cy: number): [number, number] => {
    const cv = canvas.current;
    if (!cv) return [0, 0];
    const r = cv.getBoundingClientRect();
    const z = sim.current.cam.s * (Math.max(r.width, r.height) / 1000);
    return [(cx - r.left - r.width / 2) / z + sim.current.cam.cx, (cy - r.top - r.height / 2) / z + sim.current.cam.cy];
  };
  const hit = (x: number, y: number): number | null => {
    const s = sim.current;
    const z = s.cam.s * (Math.max(window.innerWidth, window.innerHeight) / 1000);
    const pad = 22 / z;
    for (const n of agents(s)) if (Math.hypot(n.x - x, n.y - y) < 19 + pad) return n.id;
    if (Math.hypot(s.nodes[REVIEW].x - x, s.nodes[REVIEW].y - y) < 36 + pad) return REVIEW;
    if (Math.hypot(s.nodes[MAIN].x - x, s.nodes[MAIN].y - y) < 30 + pad) return MAIN;
    return null;
  };
  const choose = (id: number | null) => {
    sel.current = id;
    setSelected(id);
    setSnap(take(sim.current, id));
  };


  return (
    <div className="story">
      <div className="stage">
        <canvas
          ref={canvas}
          className="graph"
          role="img"
          aria-label="A live graph: agents claim tasks from a queue, pass a review gate and ship to main."
          onPointerDown={(e) => {
            const [x, y] = world(e.clientX, e.clientY);
            press.current = { id: hit(x, y), x: e.clientX, y: e.clientY, moved: false, touch: e.pointerType === "touch" };
            if (press.current.id !== null && !press.current.touch) e.currentTarget.setPointerCapture(e.pointerId);
          }}
          onPointerMove={(e) => {
            const p = press.current;
            if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 7) p.moved = true;
            if (p.id !== null && p.moved && !p.touch && p.id !== QUEUE && p.id !== REVIEW && p.id !== MAIN) {
              const [x, y] = world(e.clientX, e.clientY);
              sim.current = moveNode(sim.current, p.id, x, y);
            }
          }}
          onPointerUp={() => {
            const p = press.current;
            if (!p.moved) choose(p.id === selected ? null : p.id);
            else sim.current = release(sim.current);
            p.id = null;
          }}
        />
        <div className="hud" aria-hidden="true">
          <span>
            <b ref={(el) => void (hud.current.agents = el)}>12</b> agents
          </span>
          <span>
            <b ref={(el) => void (hud.current.holding = el)}>0</b> holding
          </span>
          <span>
            <b ref={(el) => void (hud.current.shipped = el)}>0</b> shipped
          </span>
          <span className="hud-ok">
            <b>0</b> collisions
          </span>
        </div>
        <span className="sr-only" aria-live="polite">
          {FILM ? "" : say}
        </span>
        {snap ? <Inspect node={snap.node} sim={snap.sim} onClose={() => choose(null)} /> : null}
      </div>

      <div className="steps">
        <section className="step" data-scene="home">
          <div className="card intro">
            <p className="eyebrow">Inside the switch</p>
            <h2>Three rules. Zero collisions.</h2>
            <p>That calm is not luck. Scroll, and the camera follows one agent through every rule Maidan enforces.</p>
            <div className="row">
              <button type="button" className="cta small" onClick={() => (sim.current = addAgent(sim.current))}>
                + Add an agent
              </button>
              <span className="hint">Tap any node to inspect it</span>
            </div>
          </div>
        </section>
        <Step scene="lease" n="1" title="A task is a lease." color="lease">
          When <b className="c-claim">Ada</b> claims a task she holds it alone, fifteen seconds at a time. Her heartbeat renews the{" "}
          <b className="c-lease">amber ring</b>. If she goes quiet the lease lapses and the task returns to the queue.
        </Step>
        <Step scene="gate" n="2" title="Nothing merges unreviewed." color="review">
          Finished work flows into the <b className="c-review">review gate</b> and moves on only with the approvals it needs. Agents can't wave
          their own work through.
        </Step>
        <Step scene="evidence" n="3" title="Nothing closes without proof." color="proof" extra={<Receipts />}>
          To reach <b className="c-proof">main</b>, a task carries its evidence: the test log, the diff, the pull request. The receipt stays for
          whoever asks later.
        </Step>
        <Step
          scene="swarm"
          n="4"
          title="Now make it a hundred."
          color="claim"
          extra={
            <>
              <span className="big">
                <b ref={(el) => void (hud.current.claims = el)}>0</b> claims · <b ref={(el) => void (hud.current.shipped2 = el)}>0</b> shipped ·{" "}
                <b className="c-proof">0</b> collisions
              </span>
              <button type="button" className="cta inline" onClick={onJoin}>
                Join the waitlist
              </button>
            </>
          }
        >
          Same three rules, a hundred agents. Footprints keep two agents off the same file, so the collision counter stays at{" "}
          <b className="c-proof">zero</b>.
        </Step>
      </div>
    </div>
  );
}

function Step({ scene, n, title, color, children, extra }: { scene: Scene; n: string; title: string; color: string; children: ReactNode; extra?: ReactNode }) {
  return (
    <section className="step" data-scene={scene}>
      <motion.div
        className="card"
        initial={{ opacity: 0, y: 24 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ amount: 0.5 }}
        transition={{ type: "spring", stiffness: 260, damping: 30 }}
      >
        <span className={`num bg-${color}`} aria-hidden="true">
          {n}
        </span>
        <h2>{title}</h2>
        <p>{children}</p>
        {extra}
      </motion.div>
    </section>
  );
}

const RECEIPTS = [
  { k: "test log", v: "214 passed · 0 failed", sha: "ci #4127" },
  { k: "diff", v: "+38 −12 · 3 files", sha: "a41f9c2" },
  { k: "pull request", v: "#1774 · approved by Rae", sha: "merged" },
];

function Receipts() {
  return (
    <motion.div className="receipts" initial="stack" whileInView="fan" viewport={{ amount: 0.8 }} aria-label="Evidence attached to the task">
      {RECEIPTS.map((r, i) => (
        <motion.div
          key={r.k}
          className="receipt"
          variants={{
            stack: { rotate: 0, x: 0, y: i * 3 },
            fan: { rotate: (i - 1) * 7, x: (i - 1) * 74, y: Math.abs(i - 1) * 8 },
          }}
          transition={{ type: "spring", stiffness: 200, damping: 22, delay: i * 0.06 }}
        >
          <span className="r-k">{r.k}</span>
          <b>{r.v}</b>
          <span className="r-sha">✓ {r.sha}</span>
        </motion.div>
      ))}
    </motion.div>
  );
}

function Inspect({ node, sim, onClose }: { node: Sim["nodes"][number]; sim: Pick<Sim, "reviewed" | "shipped" | "pulses">; onClose: () => void }) {
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
    <aside className="inspect" aria-label={`Inspect ${node.name}`}>
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
    </aside>
  );
}
