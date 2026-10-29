import { useReducedMotion } from "motion/react";
import { motion } from "motion/react";
import { useState } from "react";
import { Circuit, PRESETS, type Config } from "./circuit";
import { Page } from "./page";

const RULES = [
  { k: "No leases", t: "Leases", h: "One holder per task.", p: "A claim is a timed lease. While one agent holds it, nobody else can grab the same work. If the agent dies, the lease expires and the task returns to the queue." },
  { k: "No reviewer", t: "Review", h: "Nothing merges unseen.", p: "Every change passes a reviewer, human or agent. Footprints show what each change touches, so two agents never edit the same files at once." },
  { k: "No human", t: "Gates", h: "People keep the last word.", p: "Gates hold work until someone with the right role approves. Agents move fast; you decide what ships." },
  { k: "Starter", t: "Evidence", h: "Every close leaves proof.", p: "A task closes with evidence attached: the diff, the test log, the review. Anyone can audit how it got done." },
];

export default function App() {
  const reduce = useReducedMotion() === true;
  const [config, setConfig] = useState<Config>(PRESETS.Starter);
  const [preset, setPreset] = useState("Starter");
  const [epoch, setEpoch] = useState(0);
  const load = (k: string) => {
    setPreset(k);
    setConfig(PRESETS[k]);
    setEpoch((e) => e + 1);
    document.getElementById("build")?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
  };
  return (
    <Page
      after={
        <section className="c-sec" id="rules" aria-labelledby="rules-h">
          <p className="c-eyebrow">The four rules</p>
          <h2 id="rules-h">Take one away. Watch what breaks.</h2>
          <div className="c-rules">
            {RULES.map((r, i) => (
              <motion.article key={r.t} className="c-rule" initial={reduce ? false : { opacity: 0, y: 24 }} whileInView={{ opacity: 1, y: 0 }}
                viewport={{ once: true, margin: "-10% 0px" }} transition={{ delay: i * 0.06, type: "spring", stiffness: 220, damping: 26 }}>
                <span className="c-rule-n">0{i + 1}</span>
                <h3>{r.t}</h3>
                <p className="c-rule-h">{r.h}</p>
                <p>{r.p}</p>
                <button type="button" className="c-link" onClick={() => load(r.k)}>
                  {r.k === "Starter" ? "Run the healthy swarm" : `Try it: ${r.k.toLowerCase()}`} →
                </button>
              </motion.article>
            ))}
          </div>
        </section>
      }
    >
      <section className="c-hero" id="build" aria-labelledby="h">
        <div className="c-hero-copy">
          <p className="c-eyebrow">Open-source coordination for agent swarms</p>
          <h1 id="h" className="c-h1">Wire up a swarm.<br />Watch it ship.</h1>
          <p className="c-lede">Drag agents onto the circuit. Maidan gives them one queue, leased claims, review gates and evidence on close, so they never trip over each other.</p>
        </div>
        <div className="c-presets" role="group" aria-label="Circuits to try">
          {Object.keys(PRESETS).map((k) => (
            <button key={k} type="button" className={`c-tag${preset === k ? " is-on" : ""}`} aria-pressed={preset === k}
              onClick={() => { setPreset(k); setConfig(PRESETS[k]); setEpoch((e) => e + 1); }}>{k}</button>
          ))}
        </div>
        <Circuit key={epoch} config={config} setConfig={(c) => { setPreset(""); setConfig(c); }} reduce={reduce} />
      </section>
    </Page>
  );
}
