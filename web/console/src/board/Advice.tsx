// Advice on a task that needs review: what a decision model reads in the
// hand-off, as ten judgments, and the green, amber or red the server's rules
// draw from them. It informs the reviewer; the sheet's buttons decide.
import { useEffect, useMemo, useRef, useState } from "react";
import { ALL_CLEAR, AdviceApi, LAND_TITLE, judgmentLine, readingOf, reasonText, rowMark, sureness, usableProviders, type AdviceState, type AdvisorProvider, type Judgment, type ProviderId } from "../core/advice.ts";
import { useSession } from "../hooks/useSession.ts";

/**
 * `stamp` is the task's last change. When it moves while the sheet is open
 * (the agent posted again, or set a new result), the advice is asked for
 * again: the server answers from what it holds when the hand-off is the same,
 * and asks the model when it is not.
 */
export function Advice({ tid, stamp = "" }: { tid: string; stamp?: string }) {
  const { store, session } = useSession();
  const api = useMemo(
    () => new AdviceApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  // The providers this workspace can ask, for the switch. A read that fails
  // leaves no switch: the advice itself still comes from the default.
  const ws = session.workspaceId;
  const [usable, setUsable] = useState<AdvisorProvider[]>([]);
  useEffect(() => {
    if (!ws) return;
    let live = true;
    api.status(ws).then((s) => live && setUsable(usableProviders(s)), () => undefined);
    return () => {
      live = false;
    };
  }, [api, ws]);
  // The reviewer's choice is kept for the next task. One that can no longer
  // be asked is not sent: the server then picks.
  const [picked, setPicked] = useState<ProviderId | null>(() => storedProvider());
  const provider = usable.some((p) => p.id === picked) ? picked : null;
  const pick = (id: ProviderId) => {
    try {
      window.localStorage.setItem(PROVIDER_KEY, id);
    } catch {
      // A browser that keeps nothing still switches for this page.
    }
    setPicked(id);
  };
  // Bumped by Ask again, which asks the model afresh.
  const [attempt, setAttempt] = useState(0);
  // The answer, with the question it answers, so a new task or a new ask
  // reads as loading until its own answer arrives. A task that only changed
  // keeps showing the answer it had until the new one lands.
  const asked = `${tid}:${attempt}:${stamp}:${provider ?? ""}`;
  const [got, setGot] = useState<{ asked: string; of: string; state: AdviceState } | null>(null);
  const of = `${tid}:${attempt}:${provider ?? ""}`;
  const state: AdviceState = got && (got.asked === asked || got.of === of) ? got.state : { kind: "loading" };
  // Whether an Ask again is still owed a fresh answer. It stays owed until an
  // answer to it is accepted: a task that changes while the model is being
  // asked starts a new request, and that one must ask afresh too, or the
  // held answer would come back first and be shown in the fresh one's place.
  const refreshOwed = useRef(false);
  const askAgain = () => {
    refreshOwed.current = true;
    setAttempt((n) => n + 1);
  };

  useEffect(() => {
    let live = true;
    const afresh = refreshOwed.current;
    const accept = (next: AdviceState) => {
      if (!live) return;
      if (afresh) refreshOwed.current = false;
      setGot({ asked, of, state: next });
    };
    api.advise(tid, afresh, provider).then(accept, (e: unknown) => accept({ kind: "failed", message: e instanceof Error ? e.message : "The model did not answer." }));
    return () => {
      live = false;
    };
  }, [api, tid, attempt, asked, of, provider]);

  // Which model answered, or a choice of who to ask when there is one.
  const who = (shown: string | null) =>
    usable.length > 1 ? (
      <label className="advice-switch">
        <span className="visually-hidden">Decision model</span>
        <select value={provider ?? shown ?? usable[0].id} onChange={(e) => pick(e.target.value as ProviderId)}>
          {usable.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </label>
    ) : null;

  // Nothing to show: advice is off here, or the task has no result to read.
  if (state.kind === "off" || state.kind === "none") return null;

  if (state.kind === "no-key") {
    return (
      <section className="advice advice-quiet" aria-label="Advice">
        <p className="advice-hint">
          A decision model can read each hand-off for you. <a className="msg-link" href="#/tools/workspace">Add a key for one under Tools, Workspace.</a>
        </p>
      </section>
    );
  }

  if (state.kind === "loading") {
    return (
      <section className="advice" aria-label="Advice" aria-busy="true">
        <header className="advice-head">
          <h3>Advice</h3>
          {who(null)}
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
          {who(null)}
        </header>
        <p className="advice-wait" role="status">
          {state.message}{" "}
          <button type="button" className="link-btn" onClick={askAgain}>Ask again</button>
        </p>
      </section>
    );
  }

  const advice = state.advice;
  return (
    <section className="advice" data-land={advice.land} aria-label="Advice">
      <header className="advice-head">
        <h3>Advice</h3>
        <span className="advice-by">
          {who(advice.provider)}
          <span className="advice-model">{advice.model}</span>
        </span>
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
          All ten readings
        </summary>
        <p className="advice-legend" aria-hidden="true"><span>What the model read</span><span>How sure it is</span></p>
        <ul className="advice-rows">
          {advice.judgments.map((j) => <Row key={j.id} j={j} />)}
        </ul>
      </details>
      <div className="advice-foot">
        <p>Advice only.</p>
        <button type="button" className="chip-btn" onClick={askAgain}>Ask again</button>
      </div>
    </section>
  );
}

const PROVIDER_KEY = "maidan_advice_provider";
const PROVIDERS: readonly string[] = ["typesafe", "openai", "google"];

function storedProvider(): ProviderId | null {
  try {
    const kept = window.localStorage.getItem(PROVIDER_KEY);
    return kept && PROVIDERS.includes(kept) ? (kept as ProviderId) : null;
  } catch {
    return null;
  }
}

// Each mark is drawn inside a disc, so a concern is a filled shape and not a
// hairline: a check for clear, a bar and dot for a concern, a question mark
// for a judgment the model could not settle.
const MARK = { clear: "M7.5 12.4l3 3 6-6.6", concern: "M12 7v6M12 16.4v.4", unsure: "M9.8 9.9a2.3 2.3 0 1 1 3.6 1.9c-.8.6-1.4 1-1.4 2M12 16.6v.3" } as const;
const WORD = { clear: "", concern: "Flagged: ", unsure: "Unsure: " } as const;

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
