import { AnimatePresence, motion, useInView, useReducedMotion, type PanInfo } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { FILM, onCue } from "./film";

export const ROLES = ["planner", "coder", "reviewer", "human"] as const;
export type Role = (typeof ROLES)[number];
export type Wells = Record<Role, number | null>;
export const EMPTY: Wells = { planner: null, coder: null, reviewer: null, human: null };
const HOME: Record<Role, number> = { planner: 0, coder: 2, reviewer: 8, human: 6 };
const QUEUE = 4;
const NAME: Record<Role, string> = { planner: "Planner", coder: "Coder", reviewer: "Reviewer", human: "Human" };
const C = [13.67, 50, 86.33];
const at = (i: number): [number, number] => [C[i % 3], C[Math.floor(i / 3)]];

/** Corner points from the queue through each connected well, in role order. */
function route(w: Wells): { pts: [number, number][]; chain: Role[]; broken: Role | null } {
  const chain: Role[] = [];
  let broken: Role | null = null;
  for (const r of ROLES) {
    if (w[r] === null) { broken = r; break; }
    chain.push(r);
  }
  const pts: [number, number][] = [at(QUEUE)];
  for (const r of chain) {
    const [px, py] = pts[pts.length - 1];
    const [x, y] = at(w[r] as number);
    if (px !== x && py !== y) pts.push([x, py]);
    pts.push([x, y]);
  }
  return { pts, chain, broken };
}
const d = (pts: [number, number][]) => pts.map(([x, y], i) => `${i ? "L" : "M"}${x} ${y}`).join(" ");

export function Circuit({ wells, setWells }: { wells: Wells; setWells: (f: (w: Wells) => Wells) => void }) {
  const reduce = useReducedMotion() === true;
  const tile = useRef<HTMLDivElement>(null);
  const live = useInView(tile, { margin: "-10% 0px" });
  const refs = useRef<(HTMLDivElement | null)[]>([]);
  const [closed, setClosed] = useState(0);
  const [sweep, setSweep] = useState(0);
  const { pts, chain, broken } = route(wells);
  const placed = ROLES.filter((r) => wells[r] !== null).length;
  const complete = broken === null;
  const jammed = !complete && chain.length > 0 && placed > chain.length;
  const occupant = (i: number) => ROLES.find((r) => wells[r] === i) ?? null;

  function place(role: Role, idx?: number) {
    setSweep((s) => s + 1);
    setWells((w) => {
      const taken = (i: number) => i === QUEUE || ROLES.some((r) => w[r] === i);
      let target = idx ?? HOME[role];
      if (taken(target)) target = [HOME[role], 1, 3, 5, 7, 0, 2, 6, 8].find((i) => !taken(i)) ?? -1;
      if (target < 0) return w;
      return { ...w, [role]: target };
    });
  }
  const remove = (role: Role) => setWells((w) => ({ ...w, [role]: null }));
  function onDrop(role: Role, info: PanInfo) {
    const x = info.point.x - window.scrollX;
    const y = info.point.y - window.scrollY;
    const hit = refs.current.findIndex((el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return x >= r.left - 14 && x <= r.right + 14 && y >= r.top - 14 && y <= r.bottom + 14;
    });
    if (hit >= 0 && hit !== QUEUE && occupant(hit) === null) place(role, hit);
  }

  useEffect(() => onCue((n) => {
    const [verb, r] = n.split(":");
    if (!ROLES.includes(r as Role)) return;
    if (verb === "place") place(r as Role);
    if (verb === "remove") remove(r as Role);
  }));

  useEffect(() => {
    if (!complete || !live) return;
    const t = window.setInterval(() => setClosed((c) => c + 1), 1100);
    return () => window.clearInterval(t);
  }, [complete, live]);

  // Token keyframes along the built route.
  const seg = pts.slice(1).map((p, i) => Math.hypot(p[0] - pts[i][0], p[1] - pts[i][1]));
  const total = seg.reduce((a, b) => a + b, 0) || 1;
  let acc = 0;
  const times = [0, ...seg.map((s) => (acc += s) / total)];
  const moving = live && !reduce && chain.length > 0;
  const key = pts.map((p) => p.join(",")).join("|");

  const status = placed === 0
    ? "Drop a Planner into the steel. Drag it, or tap."
    : complete
      ? "Circuit closed. Take one away. Watch what breaks."
      : jammed
        ? `Jammed. No ${NAME[broken as Role]}, so nothing gets past the ${NAME[chain[chain.length - 1]]}.`
        : `${NAME[broken as Role]} next. The channel is cut, waiting.`;

  return (
    <div className="square-stage">
      <div className={`tile${complete ? " lit" : ""}${jammed ? " jam" : ""}`} ref={tile} role="group" aria-label={`Circuit tile, ${placed} of 4 roles placed`}>
        <span className="screw s1" aria-hidden="true" />
        <span className="screw s2" aria-hidden="true" />
        <span className="screw s3" aria-hidden="true" />
        <span className="screw s4" aria-hidden="true" />
        {sweep > 0 && !reduce ? (
          <motion.span key={sweep} className="sweep" aria-hidden="true" initial={{ x: "-120%", opacity: 0.9 }} animate={{ x: "140%", opacity: 0 }} transition={{ duration: 1.1, ease: [0.3, 0, 0.2, 1] }} />
        ) : null}
        <div className="wells">
          <svg className="channels" viewBox="0 0 100 100" aria-hidden="true">
            {[0, 1, 2].map((i) => (
              <g key={i} className="groove">
                <path d={`M${C[0]} ${C[i]} H${C[2]}`} />
                <path d={`M${C[i]} ${C[0]} V${C[2]}`} />
              </g>
            ))}
            {chain.length > 0 ? (
              <motion.path key={key} d={d(pts)} className="lit-path" initial={reduce ? false : { pathLength: 0 }} animate={{ pathLength: 1 }} transition={{ duration: 0.7, ease: [0.16, 1, 0.3, 1] }} />
            ) : null}
            {moving
              ? [0, 1, 2, 3].map((i) =>
                  complete ? (
                    <motion.circle
                      key={`${key}-${i}`}
                      r="2.6"
                      className="brass"
                      initial={{ cx: pts[0][0], cy: pts[0][1], opacity: 0 }}
                      animate={{ cx: pts.map((p) => p[0]), cy: pts.map((p) => p[1]), opacity: pts.map((_, j) => (j === 0 || j === pts.length - 1 ? 0 : 1)) }}
                      transition={{ duration: total / 34, times, ease: "linear", repeat: Infinity, delay: (i * total) / 34 / 4 }}
                    />
                  ) : (
                    <motion.circle
                      key={`${key}-${i}`}
                      r="2.6"
                      className="brass"
                      initial={{ cx: pts[0][0], cy: pts[0][1] }}
                      animate={{ cx: pts.map((p) => p[0]), cy: pts.map((p) => p[1]) }}
                      transition={{ duration: total / 34, times, ease: "linear", delay: i * 0.5 }}
                    />
                  ),
                )
              : null}
          </svg>
          {Array.from({ length: 9 }, (_, i) => {
            const r = occupant(i);
            return (
              <div className={i === QUEUE ? "well queue" : r ? "well full" : "well"} key={i} ref={(el) => { refs.current[i] = el; }}>
                {i === QUEUE ? (
                  <span className="queue-mark" aria-hidden="true"><b>{String(closed % 100).padStart(2, "0")}</b>QUEUE</span>
                ) : null}
                <AnimatePresence>
                  {r ? (
                    <motion.button
                      type="button"
                      key={r}
                      className={`puck ${r}${jammed && r === chain[chain.length - 1] ? " stuck" : ""}`}
                      aria-label={`Remove the ${NAME[r]}`}
                      onClick={() => remove(r)}
                      initial={reduce ? false : { scale: 1.6, y: -40, opacity: 0 }}
                      animate={{ scale: 1, y: 0, opacity: 1 }}
                      exit={reduce ? { opacity: 0 } : { scale: 1.3, y: -24, opacity: 0, transition: { duration: 0.25 } }}
                      transition={{ type: "spring", stiffness: 560, damping: 19, mass: 0.9 }}
                    >
                      <span className="puck-label">{r[0]}</span>
                    </motion.button>
                  ) : null}
                </AnimatePresence>
              </div>
            );
          })}
        </div>
        <p className="tile-engrave">MDN-01 · {complete ? "CLOSED" : jammed ? "JAM" : `${placed}/4`}</p>
        <span className={`lamp${complete ? " ok" : jammed ? " bad" : ""}`} aria-hidden="true" />
      </div>
      <div className="tray" aria-label="Roles to place">
        {ROLES.map((r) =>
          wells[r] !== null ? (
            <span className="slot-empty" key={r} aria-hidden="true"><span>{NAME[r]}</span></span>
          ) : (
            <span className="tray-slot" key={r}>
              <motion.button
                type="button"
                className={`puck ${r} loose`}
                drag={!reduce && !FILM}
                dragSnapToOrigin
                dragElastic={0.9}
                whileDrag={{ scale: 1.18, zIndex: 5 }}
                whileTap={{ scale: 0.94 }}
                onDragEnd={(_, info) => onDrop(r, info)}
                onClick={() => place(r)}
                aria-label={`Place the ${NAME[r]} in the circuit`}
              >
                <span className="puck-label">{r[0]}</span>
              </motion.button>
              <span className="tray-name" aria-hidden="true">{NAME[r]}</span>
            </span>
          ),
        )}
      </div>
      <p className={`hint status${jammed ? " bad" : ""}`} aria-live="polite">{status}</p>
      <p className="meter" aria-hidden="true"><span>{closed}</span> tasks closed · <span>0</span> collisions</p>
    </div>
  );
}
