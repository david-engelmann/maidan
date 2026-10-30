// Needs you, iPhone first: one column of large cards, the actions at the
// bottom of each card where a thumb reaches. A swipe right starts the yes,
// a swipe left the no; both stop at a confirm or a note, and every swipe has
// a button that does the same thing.
import { AnimatePresence, motion, useReducedMotion, type PanInfo } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  NOTE_ASK,
  NeedsApi,
  approvedLine,
  groupNeeds,
  kindLabel,
  modelRequestLine,
  needKey,
  splitBlocked,
  splitQuestion,
  waitingFor,
  type Ask,
  type GateView,
  type ConfirmLink,
  parseConfirmLink,
  type Need,
  type RowState,
} from "../core/needs.ts";
import { convHash } from "../core/threads.ts";
import { say } from "../core/notices.ts";
import { useNeeds } from "../hooks/useNeeds.ts";
import { useSession } from "../hooks/useSession.ts";

const SWIPE = 96;

function clock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function NeedsYou() {
  const { session, store } = useSession();
  const api = useMemo(
    () =>
      new NeedsApi(
        (url, init) => fetch(url, init),
        () => store.getSnapshot().credentials,
        () => store.getSnapshot().mode,
      ),
    [store],
  );
  const { data, refresh } = useNeeds(session, api);
  // Read the signed link once, then drop it from the address bar so the
  // token does not linger in history or get shared by a copied URL.
  const [link, setLink] = useState(() => parseConfirmLink(window.location.hash));
  useEffect(() => {
    if (link) window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}#/needs`);
  }, [link]);
  // A row with an answer in flight or on screen stays until it is done,
  // even when a live refresh has already dropped it from the list.
  const [held, setHeld] = useState<Map<string, Need>>(new Map());
  const hold = useCallback((it: Need, on: boolean) => {
    setHeld((h) => {
      const n = new Map(h);
      if (on) n.set(needKey(it), it);
      else n.delete(needKey(it));
      return n;
    });
  }, []);

  const shown = useMemo(() => {
    const keys = new Set(data.items.map(needKey));
    return [...data.items, ...[...held.values()].filter((it) => !keys.has(needKey(it)))];
  }, [data.items, held]);
  const { groups, headings } = useMemo(() => groupNeeds(shown), [shown]);

  if (session.status === "loading") return <p className="muted">Checking your sign-in…</p>;
  if (session.status === "signed-out")
    return (
      <div className="card">
        <h2>Sign in to see what needs you</h2>
        <p className="muted">Sign in on the classic board, then come back here.</p>
        <a className="button steel" href="/ui/">Open the classic board</a>
      </div>
    );

  return (
    <section className="needs" aria-labelledby="needs-title">
      {link ? <ConfirmPanel api={api} link={link} workspaceId={session.workspaceId} onDone={() => { setLink(null); void refresh(); }} /> : null}
      <h2 id="needs-title" className="visually-hidden">Needs you</h2>
      {data.status === "ready" && <p className="needs-count">{shown.length ? `${shown.length} waiting on you` : "Nothing waiting"}</p>}
      {data.staleSince !== null && (
        <p className="needs-stale" role="status">
          {data.error ?? `Stale since ${clock(data.staleSince)}. Reconnecting…`}
        </p>
      )}
      {data.status === "loading" && <p className="muted">Loading what needs you…</p>}
      {data.status === "error" && (
        <div className="card" role="alert">
          <p>{data.error}</p>
          <button type="button" className="btn" onClick={() => void refresh()}>Try again</button>
        </div>
      )}
      {data.status === "ready" && !shown.length && (
        <div className="card needs-empty">
          <p className="mono">All clear</p>
          <p>No reviews, approvals, blocks or questions are waiting on you.</p>
        </div>
      )}
      {groups.map((g) => (
        <div key={g.id} className="needs-group" role="group" aria-label={g.label}>
          {headings && <h3 className="mono needs-group-head">{g.label}</h3>}
          <ul className="needs-list">
            <AnimatePresence initial={false}>
              {g.items.map((it) => (
                <NeedRow
                  key={needKey(it)}
                  item={it}
                  primary={it === shown[0]}
                  gate={it.gate_id ? data.gates.get(it.gate_id) : undefined}
                  me={session.memberId}
                  api={api}
                  hold={hold}
                  onSettled={refresh}
                />
              ))}
            </AnimatePresence>
          </ul>
        </div>
      ))}
    </section>
  );
}

type RowProps = {
  item: Need;
  primary: boolean;
  gate: GateView | undefined;
  me: string | null;
  api: NeedsApi;
  hold: (it: Need, on: boolean) => void;
  onSettled: () => Promise<void>;
};

/** What the row says above its buttons. */
function rowText(it: Need, gate: GateView | undefined): { title: string; lines: string[] } {
  if (it.kind === "blocked") {
    const b = splitBlocked(it.summary);
    return { title: b.title, lines: [b.why] };
  }
  if (it.kind === "question") {
    const q = splitQuestion(it);
    return { title: q.question, lines: [`on ${q.title}`] };
  }
  if (it.kind === "open_gate") {
    const lines: string[] = [];
    const asked = modelRequestLine(gate?.model_request);
    if (asked) lines.push(asked);
    if (gate?.gate.prompt && gate.gate.prompt !== it.summary) lines.push(gate.gate.prompt);
    return { title: it.summary, lines };
  }
  return { title: it.summary, lines: it.kind === "unassigned_review" ? ["no reviewer named"] : [] };
}

const SENDING: Record<Ask, string> = {
  approve: "Approving…",
  approve_note: "Approving…",
  request_changes: "Sending back…",
  accept: "Approving…",
  decline: "Declining…",
  unblock: "Unblocking…",
  close: "Closing…",
};

const CONFIRM: Partial<Record<Ask, string>> = {
  approve: "Approve this work?",
  accept: "Approve this request?",
  unblock: "Unblock this task?",
  close: "Close this task?",
};

function NeedRow({ item, primary, gate, me, api, hold, onSettled }: RowProps) {
  const reduce = Boolean(useReducedMotion());
  const [state, setState] = useState<RowState>({ phase: "idle" });
  const [note, setNote] = useState("");
  const [root, setRoot] = useState<{ loaded: boolean; value: string | null }>({ loaded: false, value: null });
  const [owner, setOwner] = useState<{ owns: boolean; required: number | null }>({ owns: false, required: null });
  const tid = item.thread_id ?? null;
  const isReview = item.kind === "review_request" || item.kind === "unassigned_review";
  const noteRef = useRef<HTMLTextAreaElement | null>(null);
  const rowRef = useRef<HTMLLIElement | null>(null);

  // A review can only be approved against the evidence it was handed.
  useEffect(() => {
    if (!isReview || !tid) return;
    let live = true;
    api
      .reviewRoot(tid)
      .then((value) => live && setRoot({ loaded: true, value }))
      .catch(() => live && setRoot({ loaded: true, value: null }));
    if (item.kind === "unassigned_review" && me)
      void Promise.all([api.ownerOf(tid), api.reviewStatus(tid)]).then(([o, rs]) => {
        if (live) setOwner({ owns: o === me, required: rs ? rs.required_count : null });
      });
    return () => {
      live = false;
    };
  }, [api, tid, isReview, item.kind, me]);

  useEffect(() => {
    if (state.phase === "note") noteRef.current?.focus();
  }, [state.phase]);

  // A finished row leaves on its own; one that can still close the task waits.
  useEffect(() => {
    if (state.phase !== "done" || state.closable) return;
    const t = setTimeout(() => {
      hold(item, false);
      void onSettled();
    }, 1200);
    return () => clearTimeout(t);
  }, [state, hold, item, onSettled]);

  const busy = state.phase === "sending" || state.phase === "done";
  const ownerOnly = item.kind === "unassigned_review" && owner.owns;

  const run = useCallback(
    async (ask: Ask, words: string) => {
      hold(item, true);
      setState({ phase: "sending", ask });
      try {
        if (ask === "approve" || ask === "approve_note") {
          if (!tid) throw new Error("This review has no task");
          await api.review(tid, "approve", words, root.value);
          const rs = await api.reviewStatus(tid);
          setState({ phase: "done", text: approvedLine(rs), tone: "ok", closable: Boolean(rs?.approvals_met) });
        } else if (ask === "request_changes") {
          if (!tid) throw new Error("This review has no task");
          await api.review(tid, "request_changes", words, null);
          setState({ phase: "done", text: "Sent back ✓", tone: "no", closable: false });
        } else if (ask === "accept" || ask === "decline") {
          if (!gate) throw new Error("This approval is no longer pending");
          await api.answerGate(gate, ask, words);
          setState({ phase: "done", text: ask === "accept" ? "Approved ✓" : "Declined", tone: ask === "accept" ? "ok" : "no", closable: false });
        } else if (ask === "unblock") {
          if (!tid) throw new Error("This block has no task");
          await api.unblock(tid);
          setState({ phase: "done", text: "Unblocked ✓", tone: "ok", closable: false });
        } else if (ask === "close") {
          if (!tid) throw new Error("This review has no task");
          await api.close(tid);
          setState({ phase: "done", text: "Closed ✓", tone: "ok", closable: false });
        }
        setNote("");
      } catch (e) {
        setState({ phase: "error", message: e instanceof Error ? e.message : "That did not go through" });
      }
    },
    [api, gate, hold, item, root.value, tid],
  );

  const start = useCallback(
    (ask: Ask) => {
      if (busy) return;
      if (ask === "approve_note" || ask === "request_changes" || ask === "decline") setState({ phase: "note", ask });
      else setState({ phase: "confirm", ask });
    },
    [busy],
  );

  const cancel = () => {
    setState({ phase: "idle" });
    setNote("");
    hold(item, false);
    rowRef.current?.querySelector<HTMLButtonElement>(".need-actions button")?.focus();
  };

  // The yes and the no a swipe stands for, when the row has them.
  const yes: Ask | null =
    item.kind === "open_gate" ? (gate ? "accept" : null) : item.kind === "blocked" ? "unblock" : isReview && !ownerOnly && root.value ? "approve" : null;
  const no: Ask | null = item.kind === "open_gate" ? (gate ? "decline" : null) : isReview ? "request_changes" : null;

  const onDragEnd = (_e: unknown, info: PanInfo) => {
    if (info.offset.x > SWIPE && yes) start(yes);
    else if (info.offset.x < -SWIPE && no) start(no);
  };

  const { title, lines } = rowText(item, gate);
  const label = kindLabel(item.kind);
  const canSwipe = !reduce && state.phase === "idle" && Boolean(yes || no);

  return (
    <motion.li
      ref={rowRef}
      layout={!reduce}
      className={`need ${primary ? "primary" : ""} need-${item.kind} phase-${state.phase}`}
      initial={reduce ? false : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, x: state.phase === "done" && state.tone === "no" ? -40 : 40, transition: { duration: 0.38 } }}
    >
      {canSwipe && (
        <div className="need-under" aria-hidden="true">
          <span className="yes">{yes ? (yes === "unblock" ? "Unblock" : "Approve") : ""}</span>
          <span className="no">{no ? (no === "decline" ? "Decline" : "Send back") : ""}</span>
        </div>
      )}
      <motion.div
        className="need-card"
        drag={canSwipe ? "x" : false}
        dragConstraints={{ left: 0, right: 0 }}
        dragElastic={0.5}
        dragSnapToOrigin
        onDragEnd={onDragEnd}
      >
        <p className="need-meta">
          <span className={`need-kind kind-${item.kind}`}>{label}</span>
          <span className={item.overdue ? "overdue" : ""}>{waitingFor(item.age_secs)}</span>
        </p>
        <h3 className="need-title">{title}</h3>
        {lines.map((l) => (
          <p key={l} className="need-line">{l}</p>
        ))}
        {isReview && root.loaded && !root.value && <p className="need-line warn">Nothing was handed over to review yet.</p>}
        {ownerOnly && owner.required !== 0 && <p className="need-line warn">Your approval does not count on your own task: name a reviewer.</p>}

        <div className="need-state" aria-live="polite">
          {state.phase === "confirm" && (
            <div className="need-confirm">
              <p>{CONFIRM[state.ask]}</p>
              <div className="need-actions">
                <button type="button" className="btn" onClick={cancel}>Cancel</button>
                <button type="button" className="btn go" autoFocus onClick={() => void run(state.ask, "")}>
                  {state.ask === "unblock" ? "Unblock" : state.ask === "close" ? "Close task" : "Approve"}
                </button>
              </div>
            </div>
          )}
          {state.phase === "note" && (
            <form
              className="need-note"
              onSubmit={(e) => {
                e.preventDefault();
                if (NOTE_ASK[state.ask].required && !note.trim()) return;
                void run(state.ask, note);
              }}
            >
              <label htmlFor={`note-${needKey(item)}`}>{NOTE_ASK[state.ask].prompt}</label>
              <textarea id={`note-${needKey(item)}`} ref={noteRef} rows={3} value={note} onChange={(e) => setNote(e.target.value)} />
              <div className="need-actions">
                <button type="button" className="btn" onClick={cancel}>Cancel</button>
                <button type="submit" className={`btn ${state.ask === "approve_note" ? "go" : "stop"}`} disabled={NOTE_ASK[state.ask].required && !note.trim()}>
                  {NOTE_ASK[state.ask].send}
                </button>
              </div>
            </form>
          )}
          {state.phase === "sending" && <p className="need-sending">{SENDING[state.ask]}</p>}
          {state.phase === "done" && (
            <div className={`need-done tone-${state.tone}`}>
              <p>{state.text}</p>
              {state.closable && (
                <div className="need-actions">
                  <button type="button" className="btn" onClick={() => { hold(item, false); void onSettled(); }}>Later</button>
                  <button type="button" className="btn go" onClick={() => void run("close", "")}>Close task</button>
                </div>
              )}
            </div>
          )}
          {state.phase === "error" && (
            <div className="need-error" role="alert">
              <p>{state.message}</p>
              <button type="button" className="btn" onClick={cancel}>OK</button>
            </div>
          )}
        </div>

        {state.phase === "idle" && (
          <div className="need-actions">
            {isReview && !ownerOnly && (
              <>
                <button type="button" className="btn" onClick={() => start("request_changes")}>Send back</button>
                <button type="button" className="btn" disabled={!root.value} onClick={() => start("approve_note")}>Add a note</button>
                <button type="button" className="btn go" disabled={!root.value} onClick={() => start("approve")}>Approve</button>
              </>
            )}
            {ownerOnly && (
              <>
                <button type="button" className="btn" onClick={() => start("request_changes")}>Send back</button>
                {owner.required === 0 && <button type="button" className="btn go" onClick={() => start("close")}>Close without review</button>}
              </>
            )}
            {item.kind === "open_gate" &&
              (gate ? (
                <>
                  <button type="button" className="btn stop" onClick={() => start("decline")}>Decline</button>
                  <button type="button" className="btn go" onClick={() => start("accept")}>Approve</button>
                </>
              ) : (
                <a className="btn" href="/ui/">Answer on the classic board</a>
              ))}
            {item.kind === "blocked" && <button type="button" className="btn go" onClick={() => start("unblock")}>Unblock</button>}
            {item.kind === "question" && (
              <a className="btn go" href={tid ? convHash({ kind: "task", id: tid }) : "/ui/"}>{tid ? "Answer in the thread" : "Answer on the classic board"}</a>
            )}
          </div>
        )}
      </motion.div>
    </motion.li>
  );
}

function ConfirmPanel({ api, link, workspaceId, onDone }: { api: NeedsApi; link: ConfirmLink; workspaceId: string | null; onDone: () => void }) {
  const [gate, setGate] = useState<GateView | null | "loading">("loading");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void (workspaceId ? api.gates(workspaceId) : Promise.resolve(new Map<string, GateView>())).then((m) => {
      if (live) setGate(m.get(link.gateId) ?? null);
    });
    return () => {
      live = false;
    };
  }, [api, link.gateId, workspaceId]);
  const confirm = async () => {
    setBusy(true);
    setErr(null);
    try {
      await api.confirmLink(link);
      say("Approval confirmed");
      onDone();
    } catch (e) {
      setErr(e instanceof Error ? e.message : "Could not confirm");
    } finally {
      setBusy(false);
    }
  };
  const model = gate && gate !== "loading" ? modelRequestLine(gate.model_request) : null;
  return (
    <div className="card confirm-link" role="region" aria-label="Confirm an approval">
      <h2>Confirm this approval</h2>
      {gate === "loading" ? <p className="muted">Looking up the gate…</p> : gate ? <p>{gate.gate.prompt || "An approval gate"}</p> : <p className="muted">This gate is not pending in this workspace. It may already be answered.</p>}
      {model ? <p className="muted small">{model}</p> : null}
      {err ? <p className="err small" role="alert">{err}</p> : null}
      <div className="edit-actions">
        <button className="button steel" disabled={busy || gate === null} onClick={() => void confirm()}>Confirm</button>
        <button className="button ghost" disabled={busy} onClick={onDone}>Not now</button>
      </div>
    </div>
  );
}
