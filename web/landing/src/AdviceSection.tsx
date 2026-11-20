import { useId, useState, type KeyboardEvent } from "react";

// Advice from a decision model, shown on three hand-offs a visitor can switch
// between. The probabilities are what TypeSafe's Jev 1.14.0 answered for each
// hand-off on 2026-11-20, through the same ten questions the server asks; the
// advice on each card is what the server's rules draw from them.

type Land = "green" | "amber" | "red";
type Id =
  | "done" | "tests_passed" | "code_change" | "unresolved_failure" | "left_undone"
  | "asks_person" | "beyond_task" | "changes_access" | "deletes_data" | "flags_risk";
type Example = {
  tab: string;
  task: string;
  result: string;
  land: Land;
  why: string[];
  /** The probability of yes Jev gave each question. */
  p: Record<Id, number>;
  /** The judgments that kept the advice from green. */
  concerns: Id[];
};

const TITLE: Record<Land, string> = { green: "Nothing flagged", amber: "Worth a look", red: "Problem reported" };

// Each judgment as a statement, for a yes and for a no: the server sends the
// same words (crates/maidan-server/src/land_gate_advisor.rs).
const READS: [Id, string, string][] = [
  ["done", "Does what the task asks", "Does not show the task was done"],
  ["tests_passed", "Reports tests or checks passing", "Reports no passing tests"],
  ["code_change", "The task asks for a code change", "The task asks for no code change"],
  ["unresolved_failure", "Reports something still failing", "Reports nothing still failing"],
  ["left_undone", "Says part of the task was left undone", "Reports nothing left undone"],
  ["asks_person", "Asks a person a question first", "Waits on no question"],
  ["beyond_task", "Changes something the task did not ask for", "Stays within the task"],
  ["changes_access", "Changes sign-in or permissions", "Leaves sign-in and permissions alone"],
  ["deletes_data", "Deletes stored data", "Deletes no stored data"],
  ["flags_risk", "The agent warns about a risk", "The agent flags no risk"],
];

const EXAMPLES: Example[] = [
  {
    tab: "Tested change",
    task: "Rate-limit the public API",
    result: "Adds a token-bucket limiter keyed by token id on every /v1 route. 14 new tests; cargo test passed (212 passed, 0 failed).",
    land: "green",
    why: ["None of the ten judgments raised a concern."],
    p: { done: 0.95, tests_passed: 0.99, code_change: 0.99, unresolved_failure: 0.02, left_undone: 0.02, asks_person: 0.02, beyond_task: 0.04, changes_access: 0.05, deletes_data: 0.03, flags_risk: 0.02 },
    concerns: [],
  },
  {
    tab: "No tests",
    task: "Rename the billing module to invoicing",
    result: "Renamed crates/billing to crates/invoicing and updated every import, the workspace manifest and the docs.",
    land: "amber",
    why: ["The task asks for a code change, and no passing tests are reported."],
    p: { done: 0.95, tests_passed: 0.01, code_change: 0.98, unresolved_failure: 0.03, left_undone: 0.02, asks_person: 0.03, beyond_task: 0.04, changes_access: 0.02, deletes_data: 0.02, flags_risk: 0.01 },
    concerns: ["tests_passed"],
  },
  {
    tab: "Still failing",
    task: "Fix the flaky upload retry test",
    result: "Reworked the retry loop to use a fake clock. The suite still fails: the test times out in 2 of 10 runs and I could not find the cause.",
    land: "red",
    why: ["It reports something still failing, and says part of the task was left undone."],
    p: { done: 0.05, tests_passed: 0.01, code_change: 0.97, unresolved_failure: 0.99, left_undone: 0.96, asks_person: 0.06, beyond_task: 0.08, changes_access: 0.02, deletes_data: 0.02, flags_risk: 0.8 },
    concerns: ["done", "tests_passed", "unresolved_failure", "left_undone", "flags_risk"],
  },
];

/** How sure the model is of the statement shown: 2% yes is a no it is 98% sure of. */
const sure = (p: number) => `${Math.min(99, Math.round(Math.max(p, 1 - p) * 100))}% sure`;

// Drawn inside a disc: a check for a clear judgment, a bar and dot for one
// that kept the advice from "nothing flagged".
const CHECK = "M7.5 12.4l3 3 6-6.6";
const FLAG = "M12 7v6M12 16.4v.4";

const NOTES: { title: string; body: string }[] = [
  {
    title: "Ten small questions",
    body: "Does it do what the task asks? Do the tests pass? Does it change sign-in, or delete data? Jev reads the agent’s own account of the work, not the diff, so every answer can be checked against the hand-off.",
  },
  {
    title: "The decision is in code",
    body: "Rules in the server turn the ten answers into nothing flagged, worth a look or problem reported. A judgment between 30% and 70% is unsure, and an unsure one never counts as clear.",
  },
  {
    title: "Bring your own key",
    body: "On maidan.world each workspace adds its own TypeSafe API key, stored encrypted. On your own server, set one for the instance.",
  },
];

/** What a decision model reads in a hand-off, and what the server makes of it. */
export function AdviceSection() {
  const [at, setAt] = useState(1);
  const id = useId();
  const ex = EXAMPLES[at];
  const onKey = (e: KeyboardEvent) => {
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = (at + step + EXAMPLES.length) % EXAMPLES.length;
    setAt(next);
    document.getElementById(`${id}-tab-${next}`)?.focus();
  };
  return (
    <section className="advise" id="advice" tabIndex={-1} aria-labelledby="advise-title">
      <div className="advise-top">
        <p className="eyebrow">Decision models</p>
        <h2 id="advise-title">Judgments from a model.<br />Decisions in code.</h2>
        <p className="advise-lede">
          When work is handed to review, Maidan sends the hand-off (the task, the result and what
          the agent said) to Jev, TypeSafe’s decision model, with ten yes-or-no questions, and gets
          a probability for each. The model never decides. Rules in the server turn its answers
          into advice, and your reviewers and approval gates decide what lands.
        </p>
      </div>

      <div className="advise-demo">
        <p className="advise-kicker" id={`${id}-tabs`}>Try three example hand-offs</p>
        <div className="advise-tabs" role="tablist" aria-labelledby={`${id}-tabs`}>
          {EXAMPLES.map((e, i) => (
            <button
              key={e.tab}
              type="button"
              role="tab"
              id={`${id}-tab-${i}`}
              aria-selected={i === at}
              aria-controls={`${id}-panel`}
              tabIndex={i === at ? 0 : -1}
              data-land={e.land}
              onClick={() => setAt(i)}
              onKeyDown={onKey}
            >
              <i aria-hidden="true" />
              {e.tab}
              <span className="sr-only">: {TITLE[e.land].toLowerCase()}</span>
            </button>
          ))}
        </div>
        <div className="advise-card" role="tabpanel" id={`${id}-panel`} aria-labelledby={`${id}-tab-${at}`} data-land={ex.land}>
          <div className="advise-handoff">
            <p className="advise-kicker">Task</p>
            <p className="advise-task">{ex.task}</p>
            <p className="advise-kicker">The agent’s result</p>
            <p className="advise-result">“{ex.result}”</p>
          </div>
          <div className="advise-verdict">
            <p className="advise-head"><span>Advice</span><span>Jev 1.14.0 by TypeSafe</span></p>
            <p className="advise-land"><i aria-hidden="true" /><strong>{TITLE[ex.land]}</strong></p>
            {ex.why.map((w) => <p className="advise-why" key={w}>{w}</p>)}
            <p className="advise-legend" aria-hidden="true"><span>What Jev read</span><span>How sure of it</span></p>
            <ul className="advise-rows">
              {READS.map(([key, yes, no]) => {
                const p = ex.p[key];
                const reading = p >= 0.7 ? yes : no;
                const concern = ex.concerns.includes(key);
                return (
                  <li key={key} data-concern={concern || undefined} aria-label={`${concern ? "Concern: " : ""}${reading}, ${sure(p)}`}>
                    <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10" /><path d={concern ? FLAG : CHECK} /></svg>
                    <span className="advise-q" aria-hidden="true">{reading}</span>
                    <span className="advise-p" aria-hidden="true">{sure(p)}</span>
                  </li>
                );
              })}
            </ul>
          </div>
        </div>
      </div>

      <div className="advise-aside">
        <div className="advise-notes">
          {NOTES.map((n) => (
            <div className="guarantee" key={n.title}>
              <h3>{n.title}</h3>
              <p>{n.body}</p>
            </div>
          ))}
        </div>
        <div className="cta-row">
          <a className="cta" href="https://try.maidan.world">See it in the live demo</a>
          <a className="cta ghost" href="https://docs.maidan.world/docs/Decision-Models.html">How the advice works</a>
        </div>
      </div>
    </section>
  );
}
