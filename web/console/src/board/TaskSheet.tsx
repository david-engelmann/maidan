// The task sheet: a bottom sheet over the home indicator on a phone, a panel
// on the right on a wide screen. It traps focus, closes on Escape or the
// scrim, and gives focus back to the card that opened it.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { BoardApi, TRANSITION_WORDS, actionsFor, parseResult, transitionsFor, type TransitionAction, ago, bearerRefusal, chips, chromeFor, closeFacts, leaseLeft, stateLabel, type CloseFacts, type Gate, type Member, type Thread } from "../core/board.ts";
import { hasToken } from "../core/api.ts";
import { useSession } from "../hooks/useSession.ts";

type Props = {
  thread: Thread | null;
  gate?: Gate;
  members: Map<string, Member>;
  api: BoardApi;
  now: number;
  onClose: () => void;
  onChanged: () => void;
};

export function TaskSheet(props: Props) {
  const reduce = Boolean(useReducedMotion());
  return (
    <AnimatePresence>
      {props.thread && (
        <motion.div
          key="scrim"
          className="sheet-scrim"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduce ? 0 : 0.18 }}
          onClick={props.onClose}
        >
          <motion.div
            className="sheet"
            role="dialog"
            aria-modal="true"
            aria-labelledby="sheet-title"
            initial={reduce ? { opacity: 0 } : { y: 40, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { y: 40, opacity: 0 }}
            transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 420, damping: 38 }}
            onClick={(e) => e.stopPropagation()}
          >
            <SheetBody key={props.thread.id} {...props} thread={props.thread} />
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function SheetBody({ thread: th, gate, members, api, now, onClose, onChanged }: Props & { thread: Thread }) {
  const { session } = useSession();
  const box = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState<"claim" | "close" | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [facts, setFacts] = useState<CloseFacts | null>(null);
  const chrome = chromeFor(th, gate);
  const name = (id: string | null | undefined) => (id ? (members.get(id)?.name ?? id.slice(0, 8)) : "");
  const terminal = th.state === "closed" || th.state === "archived";
  const can = actionsFor(th, session.memberId, hasToken(session.credentials));

  // Focus moves in, Tab stays in, Escape closes, focus goes back on close.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    box.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== "Tab" || !box.current) return;
      const f = Array.from(box.current.querySelectorAll<HTMLElement>("button:not([disabled]), a[href], [tabindex='0']"));
      if (!f.length) return;
      const first = f[0];
      const last = f[f.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      opener?.focus();
    };
  }, [onClose]);

  // A close clears the claim, so who closed it comes from its transitions.
  useEffect(() => {
    if (!terminal) return;
    let live = true;
    api
      .transitions(th.id)
      .then((rows) => live && setFacts(closeFacts(th, rows)))
      .catch(() => live && setFacts(null));
    return () => {
      live = false;
    };
  }, [api, th, terminal]);

  const act = async (kind: "claim" | "close") => {
    setBusy(kind);
    setNote(null);
    try {
      if (kind === "claim") await api.claim(th.id);
      else await api.close(th.id);
      setNote(kind === "claim" ? "Claimed. It is yours now." : "Closed.");
      onChanged();
    } catch (e) {
      setNote(e instanceof Error ? e.message : "That did not go through");
    } finally {
      setBusy(null);
    }
  };

  const claimWhy = can.claim === false ? null : can.claim ?? bearerRefusal(session.credentials);
  return (
    <div ref={box} className="sheet-body">
      <div className="grabber" aria-hidden="true" />
      <header className="sheet-head">
        <p className="mono">{chrome.hint}</p>
        <h2 id="sheet-title">{th.title || th.id.slice(0, 8)}</h2>
        <button className="icon-btn sheet-x" aria-label="Close the task sheet" onClick={onClose} data-autofocus>
          <svg viewBox="0 0 24 24" className="icon" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
        </button>
      </header>

      <dl className="facts">
        <div><dt>State</dt><dd><span className={`state state-${chrome.lane}`}>{stateLabel(th, chrome)}</span></dd></div>
        {th.assignee_id && !terminal && <div><dt>Held by</dt><dd>{name(th.assignee_id)}{th.assignment_expires_at ? ` · ${leaseLeft(th.assignment_expires_at, now)}` : ""}</dd></div>}
        {facts && (
          <div>
            <dt>{facts.verb === "completed by" ? "Completed by" : "Closed by"}</dt>
            <dd>
              {name(facts.actorId)}
              {facts.at && (
                <>
                  {" · "}
                  <time dateTime={facts.at} title={new Date(facts.at).toLocaleString()}>{ago(facts.at, now)}</time>
                </>
              )}
            </dd>
          </div>
        )}
        {facts?.handedOffBy && <div><dt>Handed off by</dt><dd>{name(facts.handedOffBy)}</dd></div>}
        {th.updated_at && <div><dt>Last change</dt><dd><time dateTime={th.updated_at} title={new Date(th.updated_at).toLocaleString()}>{ago(th.updated_at, now)}</time></dd></div>}
      </dl>

      {chips(th, chrome.lane).length > 0 && (
        <ul className="sheet-chips" aria-label="Flags">
          {chips(th, chrome.lane).map((c) => <li key={c.kind} className={`chip chip-${c.kind}`} title={c.title}>{c.text}</li>)}
        </ul>
      )}
      {th.status?.note && <p className="muted">{th.status.note}</p>}

      <div className="sheet-actions">
        {can.claim !== false && (
          <button className="button steel" disabled={busy !== null || claimWhy !== null} aria-describedby={claimWhy ? "claim-why" : undefined} onClick={() => void act("claim")}>
            {busy === "claim" ? "Claiming…" : "Claim"}
          </button>
        )}
        {can.close && (
          <button className="button ink" disabled={busy !== null} onClick={() => void act("close")}>
            {busy === "close" ? "Closing…" : "Close task"}
          </button>
        )}
      </div>
      {claimWhy && <p id="claim-why" className="muted small">{claimWhy}</p>}
      <MoreActions th={th} api={api} onChanged={onChanged} setNote={setNote} />
      <p className="sheet-note" role="status" aria-live="polite">{note}</p>
    </div>
  );
}

function MoreActions({ th, api, onChanged, setNote }: { th: Thread; api: BoardApi; onChanged: () => void; setNote: (s: string | null) => void }) {
  const { session } = useSession();
  const moves = transitionsFor(th);
  const [move, setMove] = useState<TransitionAction | "">("");
  const [armed, setArmed] = useState(false);
  const [result, setResult] = useState("");
  const [lease, setLease] = useState("");
  const [busy, setBusy] = useState(false);
  const tokenWhy = bearerRefusal(session.credentials);
  const go = async (fn: () => Promise<unknown>, done: string) => {
    setBusy(true);
    setNote(null);
    try {
      await fn();
      setNote(done);
      onChanged();
    } catch (e) {
      setNote(e instanceof Error ? e.message : "That did not go through");
    } finally {
      setBusy(false);
      setArmed(false);
    }
  };
  const apply = () => {
    if (!move) return;
    if (move === "archive" && !armed) return setArmed(true);
    void go(() => api.transition(th.id, move), `${TRANSITION_WORDS[move]}: done.`);
  };
  const record = () => {
    const parsed = parseResult(result);
    if (!parsed.ok) return setNote(parsed.why);
    if (!lease.trim()) return setNote("Paste the claim's lease id: only the claim holder records a result.");
    void go(() => api.setResult(th.id, parsed.value, lease), "Result recorded.");
  };
  return (
    <details className="sheet-more">
      <summary>More actions</summary>
      {moves.length > 0 && (
        <div className="row-form">
          <label className="field grow">
            Move to
            <select className="select" value={move} onChange={(e) => { setMove(e.target.value as TransitionAction | ""); setArmed(false); }}>
              <option value="">Pick a state…</option>
              {moves.map((m) => <option key={m} value={m}>{TRANSITION_WORDS[m]}</option>)}
            </select>
          </label>
          <button type="button" className={armed ? "button danger" : "button steel"} disabled={!move || busy} onClick={apply}>
            {armed ? "Archive? Tap again" : "Apply"}
          </button>
        </div>
      )}
      {th.state !== "closed" && th.state !== "archived" && (
        <div className="result-form">
          <label className="field">
            Result
            <textarea rows={3} value={result} onChange={(e) => setResult(e.target.value)} placeholder='JSON, or a plain summary' />
          </label>
          <label className="field">
            Claim lease id
            <input value={lease} onChange={(e) => setLease(e.target.value)} spellCheck={false} autoCapitalize="off" className="mono" />
          </label>
          <button type="button" className="button steel" disabled={busy || tokenWhy !== null} onClick={record}>Record result</button>
          {tokenWhy && <p className="muted small">{tokenWhy}</p>}
        </div>
      )}
    </details>
  );
}
