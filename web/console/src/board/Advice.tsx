// Advice on a task that needs review: what a decision model reads in the
// hand-off, as ten judgments, and the green, amber or red the server's rules
// draw from them. It informs the reviewer; the sheet's buttons decide.
import { useEffect, useMemo, useState } from "react";
import { ALL_CLEAR, AdviceApi, LAND_TITLE, judgmentLine, modelLine, readingOf, reasonText, rowMark, sureness, type AdviceState, type Judgment } from "../core/advice.ts";
import { useSession } from "../hooks/useSession.ts";

export function Advice({ tid }: { tid: string }) {
  const { store } = useSession();
  const api = useMemo(
    () => new AdviceApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  // Bumped by Ask again, which asks the model afresh.
  const [attempt, setAttempt] = useState(0);
  // The answer, with the question it answers, so a new task or a new ask
  // reads as loading until its own answer arrives.
  const asked = `${tid}:${attempt}`;
  const [got, setGot] = useState<{ asked: string; state: AdviceState } | null>(null);
  const state: AdviceState = got?.asked === asked ? got.state : { kind: "loading" };

  useEffect(() => {
    let live = true;
    api.advise(tid, attempt > 0).then(
      (next) => live && setGot({ asked, state: next }),
      (e: unknown) => live && setGot({ asked, state: { kind: "failed", message: e instanceof Error ? e.message : "The model did not answer." } }),
    );
    return () => {
      live = false;
    };
  }, [api, tid, attempt, asked]);

  // Nothing to show: advice is off here, or the task has no result to read.
  if (state.kind === "off" || state.kind === "none") return null;

  if (state.kind === "no-key") {
    return (
      <section className="advice advice-quiet" aria-label="Advice">
        <p className="advice-hint">
          A decision model can read each hand-off for you. <a className="msg-link" href="#/tools">Add your TypeSafe key under Tools, Workspace.</a>
        </p>
      </section>
    );
  }

  if (state.kind === "loading") {
    return (
      <section className="advice" aria-label="Advice" aria-busy="true">
        <header className="advice-head">
          <h3>Advice</h3>
        </header>
        <p className="advice-wait" role="status"><span className="advice-pulse" aria-hidden="true" />Reading the hand-off…</p>
      </section>
    );
  }

  if (state.kind === "failed") {
    return (
      <section className="advice" aria-label="Advice">
        <header className="advice-head">
          <h3>Advice</h3>
        </header>
        <p className="advice-wait" role="status">
          {state.message}{" "}
          <button type="button" className="link-btn" onClick={() => setAttempt((n) => n + 1)}>Ask again</button>
        </p>
      </section>
    );
  }

  const advice = state.advice;
  return (
    <section className="advice" data-land={advice.land} aria-label="Advice">
      <header className="advice-head">
        <h3>Advice</h3>
        <span className="advice-model">{modelLine(advice)}</span>
      </header>
      <p className="advice-land">
        <span className="advice-lamp" aria-hidden="true" />
        <strong>{LAND_TITLE[advice.land]}</strong>
      </p>
      {advice.reasons.length === 0 ? (
        <p className="advice-why">{ALL_CLEAR}</p>
      ) : (
        <ul className="advice-why">
          {advice.reasons.map((r) => <li key={r}>{reasonText(r)}</li>)}
        </ul>
      )}
      <details className="advice-rest">
        <summary>
          <svg className="advice-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
          The ten judgments
        </summary>
        <p className="advice-legend" aria-hidden="true"><span>What the model read</span><span>How sure of it</span></p>
        <ul className="advice-rows">
          {advice.judgments.map((j) => <Row key={j.id} j={j} />)}
        </ul>
      </details>
      <div className="advice-foot">
        <p>Advice only: it approves and blocks nothing. You decide.</p>
        <button type="button" className="chip-btn" onClick={() => setAttempt((n) => n + 1)}>Ask again</button>
      </div>
    </section>
  );
}

// Each mark is drawn inside a disc, so a concern is a filled shape and not a
// hairline: a check for clear, a bar and dot for a concern, a question mark
// for a judgment the model could not settle.
const MARK = { clear: "M7.5 12.4l3 3 6-6.6", concern: "M12 7v6M12 16.4v.4", unsure: "M9.8 9.9a2.3 2.3 0 1 1 3.6 1.9c-.8.6-1.4 1-1.4 2M12 16.6v.3" } as const;
const WORD = { clear: "", concern: "Concern: ", unsure: "Unsure: " } as const;

function Row({ j }: { j: Judgment }) {
  const mark = rowMark(j);
  return (
    <li className="advice-row" data-mark={mark} aria-label={WORD[mark] + judgmentLine(j)}>
      <svg className="advice-mark" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10" /><path d={MARK[mark]} /></svg>
      <span className="advice-q" aria-hidden="true">{readingOf(j)}</span>
      <span className="advice-p" aria-hidden="true">{sureness(j)}</span>
    </li>
  );
}
