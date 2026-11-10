// Cancel a task, or close it as abandoned, in two steps: say why, then read
// what ending it does and confirm. The task sheet and the full thread view
// both use it, so the words and the checks are the same in each.
import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  CANCEL_REASON_MAX,
  CLOSE_REASON_WORDS,
  END_TITLE,
  EndTaskApi,
  cancelReasonRefusal,
  closeRefusal,
  endActionsFor,
  endEffects,
  type CloseReason,
  type EndKind,
} from "../core/endTask.ts";
import type { Thread } from "../core/board.ts";
import { useSession } from "../hooks/useSession.ts";

/** Which end actions the signed-in caller may take on `th`. */
export function useEndActions(th: Thread | null | undefined): { cancel: boolean; abandon: boolean } {
  const { session } = useSession();
  if (!th) return { cancel: false, abandon: false };
  return endActionsFor(th, session.memberId, session.grant?.capabilities ?? null);
}

/** The writes, bound to this tab's credential. */
export function useEndTaskApi(): EndTaskApi {
  const { store } = useSession();
  return useMemo(
    () => new EndTaskApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
}

type Props = {
  th: Thread;
  kind: EndKind;
  api: EndTaskApi;
  /** After the server accepted: the sentence to show, and the task as it is now. */
  onDone: (said: string, th: Thread) => void;
  /** Leave without ending the task. */
  onBack: () => void;
};

const CONFIRM_WORD: Record<EndKind, string> = { cancel: "Cancel task", abandon: "Close task" };
const WORKING_WORD: Record<EndKind, string> = { cancel: "Cancelling…", abandon: "Closing…" };

export function EndTaskForm({ th, kind, api, onDone, onBack }: Props) {
  const id = useId();
  const [step, setStep] = useState<"say" | "confirm">("say");
  const [reason, setReason] = useState("");
  const [why, setWhy] = useState<CloseReason>("abandoned");
  const [other, setOther] = useState("");
  const [working, setWorking] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const first = useRef<HTMLTextAreaElement & HTMLInputElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    if (step === "say") first.current?.focus({ preventScroll: true });
    else heading.current?.focus({ preventScroll: true });
  }, [step]);

  const refusal = kind === "cancel" ? cancelReasonRefusal(reason) : closeRefusal(why, other, th.id);
  const next = () => {
    setProblem(refusal);
    if (!refusal) setStep("confirm");
  };
  const send = async () => {
    if (working) return;
    setWorking(true);
    setProblem(null);
    try {
      if (kind === "cancel") {
        const out = await api.cancel(th.id, reason);
        onDone(out.replayed ? "It was already cancelled." : "Cancelled. The task is closed.", out.thread);
      } else {
        const out = await api.closeAbandoned(th.id, why, other);
        onDone("Closed. Nothing landed.", out);
      }
    } catch (e) {
      setProblem(e instanceof Error && e.message ? e.message : "That did not go through. Try again.");
    } finally {
      setWorking(false);
    }
  };

  const title = th.title?.trim() || "this task";
  return (
    <section className="end-task" aria-labelledby={`${id}-h`}>
      {step === "say" ? (
        <>
          <h3 id={`${id}-h`} className="end-task-title">{END_TITLE[kind]}</h3>
          {kind === "cancel" ? (
            <label className="field">
              Why cancel it?
              <textarea
                ref={first}
                rows={2}
                maxLength={CANCEL_REASON_MAX}
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                placeholder="Nobody needs this any more"
                aria-describedby={`${id}-hint`}
              />
            </label>
          ) : (
            <fieldset className="end-reasons">
              <legend>Why close it?</legend>
              {(Object.keys(CLOSE_REASON_WORDS) as CloseReason[]).map((r, i) => (
                <label key={r} className="check">
                  <input
                    ref={i === 0 ? first : undefined}
                    type="radio"
                    name={`${id}-why`}
                    value={r}
                    checked={why === r}
                    onChange={() => setWhy(r)}
                  />
                  {CLOSE_REASON_WORDS[r]}
                </label>
              ))}
              {why !== "abandoned" && (
                <label className="field">
                  {why === "duplicate" ? "The task that covers it (link or id)" : "The task that replaces it (link or id, optional)"}
                  <input value={other} onChange={(e) => setOther(e.target.value)} spellCheck={false} autoCapitalize="off" />
                </label>
              )}
            </fieldset>
          )}
          <p id={`${id}-hint`} className="muted small">
            {kind === "cancel" ? "Posted on the task, and on its pull request if it has one." : "Kept with the task, and on its pull request if it has one."}
          </p>
          <div className="end-task-actions">
            <button type="button" className="button ink" onClick={next}>Continue</button>
            <button type="button" className="button ghost" onClick={onBack}>Keep the task</button>
          </div>
        </>
      ) : (
        <>
          <h3 id={`${id}-h`} ref={heading} tabIndex={-1} className="end-task-title">
            {kind === "cancel" ? `Cancel “${title}”?` : `Close “${title}” without landing it?`}
          </h3>
          <p className="small">What happens:</p>
          <ul className="end-effects">
            {endEffects(kind, why).map((line) => <li key={line}>{line}</li>)}
          </ul>
          <div className="end-task-actions">
            <button type="button" className="button ghost danger" disabled={working} onClick={() => void send()}>
              {working ? WORKING_WORD[kind] : CONFIRM_WORD[kind]}
            </button>
            <button type="button" className="button ghost" disabled={working} onClick={() => setStep("say")}>Go back</button>
          </div>
        </>
      )}
      <p className="end-task-problem small" role="alert">{problem}</p>
    </section>
  );
}
