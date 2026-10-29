import { useEffect, useRef, useState, type PointerEvent as RPointerEvent } from "react";

export type Role = "Planner" | "Coder" | "Reviewer" | "Human";
type StationId = "queue" | "plan" | "build" | "review" | "approve" | "ship";
type Station = { id: StationId; label: string; sub: string; accepts?: Role; cap: number };

const STATIONS: Station[] = [
  { id: "queue", label: "Queue", sub: "one shared list", cap: 0 },
  { id: "plan", label: "Plan", sub: "scope the task", accepts: "Planner", cap: 1 },
  { id: "build", label: "Claim lease", sub: "one holder per task", accepts: "Coder", cap: 2 },
  { id: "review", label: "Review", sub: "a second pair of eyes", accepts: "Reviewer", cap: 1 },
  { id: "approve", label: "Gate", sub: "a human signs off", accepts: "Human", cap: 1 },
  { id: "ship", label: "Ship", sub: "closed with evidence", cap: 0 },
];

export const ROLE_COLOR: Record<Role, string> = {
  Planner: "#a78bfa",
  Coder: "#67e8f9",
  Reviewer: "#fcd34d",
  Human: "#f9a8d4",
};

export type Config = { plan: number; build: number; review: number; approve: number; lease: boolean };
export const PRESETS: Record<string, Config> = {
  Starter: { plan: 1, build: 2, review: 1, approve: 1, lease: true },
  "No leases": { plan: 1, build: 2, review: 1, approve: 1, lease: false },
  "No reviewer": { plan: 1, build: 2, review: 0, approve: 1, lease: true },
  "No human": { plan: 1, build: 2, review: 1, approve: 0, lease: true },
  Empty: { plan: 0, build: 0, review: 0, approve: 0, lease: true },
};

type Kind = "ok" | "dup" | "bug";
type Packet = { id: number; seg: number; p: number; wait: number; kind: Kind; twin?: boolean };
type Stats = { shipped: number; wasted: number; unreviewed: number; waiting: number };

function diagnose(c: Config) {
  const out: { tone: "bad" | "good"; text: string }[] = [];
  if (!c.plan) out.push({ tone: "bad", text: "No planner: work arrives unscoped and builds crawl." });
  if (!c.build) out.push({ tone: "bad", text: "No coder: nobody claims work and the queue piles up." });
  if (c.build >= 2 && !c.lease) out.push({ tone: "bad", text: "Leases off: two coders grab the same task. Half the work is wasted." });
  if (!c.review) out.push({ tone: "bad", text: "No reviewer: changes skip review and bugs reach Ship." });
  if (!c.approve) out.push({ tone: "bad", text: "No human at the gate: finished work waits forever." });
  if (!out.length) out.push({ tone: "good", text: "Healthy swarm: one holder per task, every change reviewed, evidence on close." });
  return out;
}

const SPEED = 0.85;
type Log = { id: number; text: string; tone: "ok" | "bad" | "warn" };
type Sim = { packets: Packet[]; next: number; spawn: number; flash: number; stats: Stats; log: Log[]; seq: number };
function say(s: Sim, text: string, tone: Log["tone"] = "ok") {
  s.log = [{ id: s.seq++, text, tone }, ...s.log].slice(0, 4);
}
const fresh = (): Sim => ({ packets: [], next: 1, spawn: 0, flash: 0, stats: { shipped: 0, wasted: 0, unreviewed: 0, waiting: 0 }, log: [], seq: 1 });

function arrive(sim: Sim, c: Config, pk: Packet, s: Station, born: Packet[]) {
  const st = sim.stats;
  if (s.id === "plan") pk.wait = c.plan ? 0.35 : 2.4;
  else if (s.id === "build") {
    if (!c.build) pk.wait = Infinity;
    else {
      pk.wait = c.plan ? 0.7 : 2.2;
      if (c.lease) say(sim, `#${pk.id} claimed: lease held by one coder`);
      if (c.build >= 2 && !c.lease && pk.kind === "ok" && !pk.twin) {
        born.push({ id: sim.next++, seg: pk.seg, p: -1, wait: pk.wait + 0.25, kind: "dup", twin: true });
        st.wasted++;
        say(sim, `#${pk.id} grabbed by two coders: duplicate work`, "bad");
      }
    }
  } else if (s.id === "review") {
    if (c.review) pk.wait = 0.5;
    else {
      pk.wait = 0;
      if (pk.kind === "ok") {
        pk.kind = "bug";
        st.unreviewed++;
        say(sim, `#${pk.id} skipped review`, "warn");
      }
    }
  } else if (s.id === "approve") {
    pk.wait = c.approve ? 0.45 : Infinity;
    if (!c.approve) say(sim, `#${pk.id} held at the gate: no human to approve`, "warn");
  }
  else if (s.id === "ship") {
    pk.wait = 0.25;
    if (pk.kind !== "dup") st.shipped++;
    if (pk.kind === "ok") say(sim, `#${pk.id} shipped with evidence`);
    else if (pk.kind === "bug") say(sim, `#${pk.id} shipped unreviewed`, "warn");
    else say(sim, `#${pk.id} duplicate thrown away at merge`, "bad");
    sim.flash = pk.id;
  }
}

function stepSim(s: Sim, c: Config, dt: number) {
  s.spawn -= dt;
  const inQueue = s.packets.filter((p) => p.seg === 0 && p.p < 0).length;
  if (s.spawn <= 0 && s.packets.length < 16 && inQueue < 4) {
    s.packets.push({ id: s.next++, seg: 0, p: -1, wait: 0.3, kind: "ok" });
    s.spawn = 1.15;
  }
  const born: Packet[] = [];
  const keep: Packet[] = [];
  for (const pk of s.packets) {
    if (pk.p < 0) {
      // Gated stations re-check config each frame so dropping a role in unblocks them live.
      const here = STATIONS[pk.seg];
      if (pk.wait === Infinity) {
        if ((here.id === "build" && c.build) || (here.id === "approve" && c.approve)) pk.wait = 0.4;
      }
      pk.wait -= dt;
      if (pk.wait <= 0) {
        if (pk.seg === STATIONS.length - 1) continue;
        pk.p = 0;
      }
      keep.push(pk);
      continue;
    }
    pk.p += dt * SPEED * (pk.kind === "dup" ? 0.92 : 1);
    if (pk.p >= 1) {
      pk.seg += 1;
      pk.p = -1;
      arrive(s, c, pk, STATIONS[pk.seg], born);
    }
    keep.push(pk);
  }
  s.packets = keep.concat(born);
  s.stats.waiting = s.packets.filter((p) => p.p < 0 && p.wait === Infinity).length;
}



export function Circuit({ config, setConfig, reduce }: { config: Config; setConfig: (c: Config) => void; reduce: boolean }) {
  const wrap = useRef<HTMLDivElement>(null);
  const [wide, setWide] = useState(false);
  const [running, setRunning] = useState(!reduce);
  const [packets, setPackets] = useState<Packet[]>([]);
  const [stats, setStats] = useState<Stats>({ shipped: 0, wasted: 0, unreviewed: 0, waiting: 0 });
  const [log, setLog] = useState<Log[]>([]);
  const [flash, setFlash] = useState<{ id: number; st: StationId } | null>(null);
  const [drag, setDrag] = useState<{ role: Role; x: number; y: number; moved: boolean } | null>(null);
  const [armed, setArmed] = useState<Role | null>(null);
  const [hover, setHover] = useState<StationId | null>(null);
  const sim = useRef<Sim>(fresh());
  const cfg = useRef(config);
  useEffect(() => {
    cfg.current = config;
  }, [config]);

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWide(e.contentRect.width >= 720));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (!running) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const sm = sim.current;
      const before = sm.flash;
      stepSim(sm, cfg.current, dt);
      if (sm.flash !== before) setFlash({ id: sm.flash, st: "ship" });
      setPackets(sm.packets.map((p) => ({ ...p })));
      setStats({ ...sm.stats });
      setLog(sm.log);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [running]);

  const W = wide ? 960 : 360;
  const H = wide ? 250 : 760;
  const pos = (i: number) => (wide ? { x: 80 + i * 160, y: 118 } : { x: 150, y: 56 + i * 112 });

  function packetXY(pk: Packet, all: Packet[], lag = 0) {
    if (pk.p < 0) {
      const a = pos(pk.seg);
      const stack = all.filter((o) => o.p < 0 && o.seg === pk.seg && o.id < pk.id).length;
      const off = Math.min(stack, 6) * 10;
      return wide ? { x: a.x - 25 + off, y: a.y + 48 } : { x: a.x + 64 + off, y: a.y };
    }
    const a = pos(pk.seg);
    const b = pos(pk.seg + 1);
    const q = Math.max(0, pk.p - lag);
    const t = q * q * (3 - 2 * q);
    const lane = pk.kind === "dup" ? 9 : 0;
    return wide ? { x: a.x + (b.x - a.x) * t, y: a.y + lane } : { x: a.x + lane, y: a.y + (b.y - a.y) * t };
  }

  function count(id: StationId) {
    return id === "plan" ? config.plan : id === "build" ? config.build : id === "review" ? config.review : id === "approve" ? config.approve : 0;
  }
  function place(role: Role, id: StationId) {
    const s = STATIONS.find((x) => x.id === id);
    if (!s || s.accepts !== role || count(id) >= s.cap) return false;
    setConfig({ ...config, [id]: count(id) + 1 });
    return true;
  }
  function remove(id: StationId) {
    if (count(id) > 0) setConfig({ ...config, [id]: count(id) - 1 });
  }

  function stationAt(x: number, y: number): StationId | null {
    let best: StationId | null = null;
    let d = 70;
    for (const el of document.querySelectorAll("[data-station] .cc-node")) {
      const r = el.getBoundingClientRect();
      const dist = Math.hypot(x - (r.left + r.width / 2), y - (r.top + r.height / 2));
      if (dist < d) {
        d = dist;
        best = el.parentElement?.getAttribute("data-station") as StationId;
      }
    }
    return best;
  }
  function onDown(role: Role, e: RPointerEvent<HTMLButtonElement>) {
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ role, x: e.clientX, y: e.clientY, moved: false });
  }
  function onMove(e: RPointerEvent<HTMLButtonElement>) {
    if (!drag) return;
    const moved = drag.moved || Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 6;
    setDrag({ ...drag, x: e.clientX, y: e.clientY, moved });
    setHover(stationAt(e.clientX, e.clientY));
  }
  function onUp(e: RPointerEvent<HTMLButtonElement>) {
    if (!drag) return;
    if (drag.moved) {
      const id = stationAt(e.clientX, e.clientY);
      if (id) place(drag.role, id);
      setArmed(null);
    } else setArmed(armed === drag.role ? null : drag.role);
    setDrag(null);
    setHover(null);
  }

  const busy = new Set(packets.filter((p) => p.p < 0 && p.wait !== Infinity && p.seg > 0).map((p) => p.seg));
  const active = drag?.moved ? drag.role : armed;
  const diag = diagnose(config);
  const healthy = diag[0].tone === "good";

  return (
    <div className="cc" ref={wrap}>
      <div className="cc-palette" role="toolbar" aria-label="Agents to drag onto the circuit">
        {(Object.keys(ROLE_COLOR) as Role[]).map((r) => (
          <button
            key={r}
            type="button"
            className={`cc-chip${armed === r ? " is-armed" : ""}`}
            style={{ ["--c" as string]: ROLE_COLOR[r] }}
            aria-pressed={armed === r}
            aria-label={`${r}: drag onto a matching station, or tap then tap a station`}
            onPointerDown={(e) => onDown(r, e)}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerCancel={() => setDrag(null)}
          >
            <span className="cc-dot" aria-hidden="true" />
            {r}
          </button>
        ))}
      </div>
      <div className={`cc-board${healthy ? " is-healthy" : ""}`}>
        <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label="A swarm circuit: Queue, Plan, Claim lease, Review, Gate, Ship" className="cc-svg">
          <defs>
            <radialGradient id="glow-ok"><stop offset="0" stopColor="#eafff6" /><stop offset="0.35" stopColor="#6ee7b7" /><stop offset="1" stopColor="#6ee7b7" stopOpacity="0" /></radialGradient>
            <radialGradient id="glow-dup"><stop offset="0" stopColor="#ffe4e6" /><stop offset="0.35" stopColor="#fb7185" /><stop offset="1" stopColor="#fb7185" stopOpacity="0" /></radialGradient>
            <radialGradient id="glow-bug"><stop offset="0" stopColor="#fff7d6" /><stop offset="0.35" stopColor="#fcd34d" /><stop offset="1" stopColor="#fcd34d" stopOpacity="0" /></radialGradient>
            <linearGradient id="wire" x1="0" x2={wide ? "1" : "0"} y1="0" y2={wide ? "0" : "1"}>
              <stop offset="0" stopColor="#67e8f9" stopOpacity="0.7" /><stop offset="1" stopColor="#a78bfa" stopOpacity="0.7" />
            </linearGradient>
          </defs>
          {STATIONS.slice(0, -1).map((s, i) => {
            const a = pos(i);
            const b = pos(i + 1);
            return (
              <g key={s.id}>
                <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} stroke="rgba(255,255,255,0.08)" strokeWidth="10" strokeLinecap="round" />
                <line x1={a.x} y1={a.y} x2={b.x} y2={b.y} stroke="url(#wire)" strokeWidth="2" className={running ? "cc-flow" : ""} />
              </g>
            );
          })}
          {STATIONS.map((s, i) => {
            const p = pos(i);
            const n = count(s.id);
            const want = active && s.accepts === active && n < s.cap;
            const empty = s.accepts && n === 0;
            const lit = flash?.st === s.id;
            return (
              <g key={s.id} data-station={s.id} transform={`translate(${p.x} ${p.y})`}
                className={`cc-st${want ? " is-target" : ""}${hover === s.id && want ? " is-hover" : ""}${empty ? " is-empty" : ""}${lit ? " is-lit" : ""}`}
                onClick={() => { if (armed && place(armed, s.id)) setArmed(null); }}>
                {busy.has(i) && <rect x="-52" y="-32" width="104" height="64" rx="20" className="cc-halo" />}
                <rect x="-46" y="-26" width="92" height="52" rx="16" className="cc-node" />
                <text y="-3" textAnchor="middle" className="cc-label">{s.label}</text>
                <text y="13" textAnchor="middle" className="cc-sub">{s.sub}</text>
                {s.accepts && Array.from({ length: s.cap }).map((_, k) => {
                  const filled = k < n;
                  const sx = wide ? -14 + k * 28 - (s.cap - 1) * 0 : -70 - k * 30;
                  const sy = wide ? -52 : 0;
                  return (
                    <g key={k} transform={`translate(${wide ? sx + (s.cap === 1 ? 14 : 0) : sx} ${sy})`}
                      role={filled ? "button" : undefined} tabIndex={filled ? 0 : undefined}
                      aria-label={filled ? `Remove ${s.accepts} from ${s.label}` : undefined}
                      onClick={(e) => { if (filled) { e.stopPropagation(); remove(s.id); } }}
                      onKeyDown={(e) => { if (filled && (e.key === "Enter" || e.key === " ")) remove(s.id); }}
                      className={filled ? "cc-slot is-filled" : "cc-slot"}>
                      <circle r="12" style={{ ["--c" as string]: ROLE_COLOR[s.accepts as Role] }} />
                      <text y="4" textAnchor="middle">{filled ? (s.accepts as string)[0] : "+"}</text>
                    </g>
                  );
                })}
                {s.id === "build" && (
                  <g transform={wide ? "translate(0 48)" : "translate(-46 34)"} role="switch" aria-checked={config.lease} tabIndex={0}
                    aria-label="Leases" className={`cc-lease${config.lease ? " is-on" : ""}`}
                    onClick={(e) => { e.stopPropagation(); setConfig({ ...config, lease: !config.lease }); }}
                    onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") setConfig({ ...config, lease: !config.lease }); }}>
                    <rect x={wide ? -34 : 0} y="-10" width="68" height="20" rx="10" />
                    <text x={wide ? 0 : 34} y="4" textAnchor="middle">{config.lease ? "lease on" : "lease off"}</text>
                  </g>
                )}
              </g>
            );
          })}
          {flash && <circle key={flash.id} cx={pos(5).x} cy={pos(5).y} r={34} className="cc-burst" />}
          <g className="cc-packets">
          {packets.map((pk) => {
            const { x, y } = packetXY(pk, packets);
            const tail = pk.p > 0.1 ? packetXY(pk, packets, 0.09) : null;
            return (
              <g key={pk.id}>
                {tail && <circle cx={tail.x} cy={tail.y} r={7} fill={`url(#glow-${pk.kind})`} opacity={0.4} />}
                <circle cx={x} cy={y} r={pk.p < 0 ? 9 : 12} fill={`url(#glow-${pk.kind})`} />
              </g>
            );
          })}
          </g>
        </svg>
        <div className="cc-hud" aria-live="polite">
          <span><b>{stats.shipped}</b> shipped</span>
          <span className={stats.wasted ? "is-bad" : ""}><b>{stats.wasted}</b> wasted</span>
          <span className={stats.unreviewed ? "is-warn" : ""}><b>{stats.unreviewed}</b> unreviewed</span>
          <span className={stats.waiting ? "is-warn" : ""}><b>{stats.waiting}</b> stuck</span>
        </div>
      </div>
      <div className="cc-side">
      <div className="cc-controls">
        <button type="button" className="c-btn c-btn-primary cc-run" onClick={() => setRunning(!running)} aria-pressed={running}>
          {running ? "Pause" : "Run"}
        </button>
        <button type="button" className="c-btn" onClick={() => { sim.current = fresh(); setPackets([]); setStats({ shipped: 0, wasted: 0, unreviewed: 0, waiting: 0 }); }}>Reset</button>
      </div>
      <ol className="cc-log" aria-label="Live ledger">
        {!log.length && <li className="is-idle"><span className="cc-led" aria-hidden="true" />Ledger: every claim, review and close lands here.</li>}
        {log.map((l) => <li key={l.id} className={`is-${l.tone}`}><span className="cc-led" aria-hidden="true" />{l.text}</li>)}
      </ol>
      <ul className="cc-diag" aria-live="polite">
        {diag.map((d) => <li key={d.text} className={`is-${d.tone}`}>{d.text}</li>)}
      </ul>
      </div>
      {drag?.moved && (
        <div className="cc-ghost" aria-hidden="true" style={{ transform: `translate(${drag.x - 44}px, ${drag.y - 22}px)`, ["--c" as string]: ROLE_COLOR[drag.role] }}>
          <span className="cc-dot" />{drag.role}
        </div>
      )}
    </div>
  );
}
