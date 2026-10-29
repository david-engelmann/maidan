import { motion, useReducedMotion } from "motion/react";
import { useRef, useState, type ReactNode } from "react";
import { SimCanvas } from "./canvas";
import { footSim, gateSim, swarmSim, type Foot, type Gate } from "./minis";
import { Page } from "./page";
import type { World } from "./swarm";

const HERO = swarmSim(36, 20);
const LEASE = swarmSim(7, 6);

function Switch({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} className={`c-switch${on ? " is-on" : ""}`} onClick={() => onChange(!on)}>
      <span className="c-switch-track" aria-hidden="true"><motion.span layout className="c-switch-knob" transition={{ type: "spring", stiffness: 500, damping: 32 }} /></span>
      <span>{label}</span>
    </button>
  );
}

function poke(el: HTMLElement | null) {
  el?.closest("section")?.querySelector("canvas")?.dispatchEvent(new Event("sim:poke"));
}

export default function App() {
  const reduce = useReducedMotion() === true;
  const world = useRef<World | null>(null);
  const [on, setOn] = useState(false);
  const [hud, setHud] = useState({ shipped: 0, waste: 0 });
  const flip = (v: boolean, el: HTMLElement | null) => {
    setOn(v);
    if (world.current) {
      world.current.order = v;
      world.current.shipped = 0;
      world.current.wasted = 0;
      world.current.work = 0;
    }
    poke(el);
  };
  return (
    <Page after={<Rules reduce={reduce} />}>
      <section className="c-hero2" aria-labelledby="h">
        <SimCanvas sim={HERO} stateRef={world} reduce={reduce} className="c-field" label={on ? "Agents claim tasks from one ordered square, one agent per task" : "Agents swarm the same tasks at random and collide"}
          onFrame={(s) => setHud({ shipped: s.shipped, waste: s.work ? Math.round((s.wasted / s.work) * 100) : 0 })} />
        <div className="c-hero2-copy">
          <p className="c-eyebrow">Open-source coordination for agent swarms</p>
          <h1 id="h" className="c-h1">{on ? <>Same agents.<br />One square.</> : <>36 agents.<br />No coordination.</>}</h1>
          <p className="c-lede">{on ? "Every task has one holder, a lease, and a place in the queue. Nothing is done twice." : "They chase the same tasks, redo each other's work, and step on each other. Red means two agents on one task."}</p>
        </div>
        <div className="c-dock" aria-live="polite">
          <div className="c-dock-stats">
            <span><b>{hud.shipped}</b> shipped</span>
            <span className={!on && hud.waste ? "is-bad" : ""}><b>{hud.waste}%</b> effort wasted</span>
          </div>
          <div ref={(el) => { dockRef.current = el; }}>
            <Switch on={on} onChange={(v) => flip(v, dockRef.current)} label="Turn on Maidan" />
          </div>
        </div>
      </section>
    </Page>
  );
}
const dockRef: { current: HTMLDivElement | null } = { current: null };

function Rule({ n, title, lead, body, children }: { n: string; title: string; lead: string; body: string; children: ReactNode }) {
  return (
    <motion.section className="c-mini" initial={{ opacity: 0, y: 40 }} whileInView={{ opacity: 1, y: 0 }} viewport={{ once: true, margin: "-15% 0px" }} transition={{ type: "spring", stiffness: 160, damping: 24 }}>
      <div className="c-mini-copy">
        <span className="c-rule-n">Rule {n}</span>
        <h3>{title}</h3>
        <p className="c-rule-h">{lead}</p>
        <p>{body}</p>
      </div>
      <div className="c-mini-stage">{children}</div>
    </motion.section>
  );
}

function Rules({ reduce }: { reduce: boolean }) {
  const lease = useRef<World | null>(null);
  const foot = useRef<Foot | null>(null);
  const gate = useRef<Gate | null>(null);
  const [l, setL] = useState({ on: true, waste: 0 });
  const [f, setF] = useState({ on: true, n: 0 });
  const [g, setG] = useState({ on: true, caught: 0, inc: 0 });
  return (
    <div className="c-sec" id="rules">
      <p className="c-eyebrow">The rules of the square</p>
      <h2>Turn a rule off.<br />Watch what breaks.</h2>
      <Rule n="01" title="Leases" lead="One holder per task." body="Claiming a task takes a timed lease. While it is held, nobody else can take it. If the holder goes quiet, the lease expires and the task goes back in the queue.">
        <SimCanvas sim={LEASE} stateRef={lease} reduce={reduce} className="c-mini-canvas" label="Seven agents and six tasks"
          onFrame={(s) => { if (s.order !== l.on) s.order = l.on; setL((p) => ({ ...p, waste: s.work ? Math.round((s.wasted / s.work) * 100) : 0 })); }} />
        <div className="c-mini-bar">
          <Switch on={l.on} label="Leases" onChange={(v) => { setL({ on: v, waste: 0 }); if (lease.current) { lease.current.order = v; lease.current.wasted = 0; lease.current.work = 0; } }} />
          <span className={l.waste ? "is-bad" : ""}><b>{l.waste}%</b> duplicated</span>
        </div>
      </Rule>
      <Rule n="02" title="Footprints" lead="Two agents never edit the same thing." body="Each claim declares the files it touches. A footprint that overlaps someone else's waits its turn instead of creating a merge conflict.">
        <SimCanvas sim={footSim} stateRef={foot} reduce={reduce} className="c-mini-canvas" label="Three agents moving blocks of files on a grid"
          onFrame={(s) => setF((p) => (p.n === s.conflicts ? p : { ...p, n: s.conflicts }))} />
        <div className="c-mini-bar">
          <Switch on={f.on} label="Footprints" onChange={(v) => { setF({ on: v, n: 0 }); if (foot.current) { foot.current.on = v; foot.current.conflicts = 0; } }} />
          <span className={f.n ? "is-bad" : ""}><b>{f.n}</b> conflicts</span>
        </div>
      </Rule>
      <Rule n="03" title="Review gates" lead="Nothing ships unseen." body="Changes stop at a gate until a reviewer, agent or human, approves. Broken work goes back instead of out. Every close carries its evidence: the diff, the tests, the review.">
        <SimCanvas sim={gateSim} stateRef={gate} reduce={reduce} className="c-mini-canvas is-short" label="Changes moving toward production through a review gate"
          onFrame={(s) => setG((p) => (p.caught === s.caught && p.inc === s.incidents ? p : { ...p, caught: s.caught, inc: s.incidents }))} />
        <div className="c-mini-bar">
          <Switch on={g.on} label="Review gate" onChange={(v) => { setG({ on: v, caught: 0, inc: 0 }); if (gate.current) { gate.current.on = v; gate.current.caught = 0; gate.current.incidents = 0; } }} />
          <span><b>{g.caught}</b> caught</span>
          <span className={g.inc ? "is-bad" : ""}><b>{g.inc}</b> incidents</span>
        </div>
      </Rule>
    </div>
  );
}
