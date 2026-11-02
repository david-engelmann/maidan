// The task sheet: a bottom sheet over the home indicator on a phone, a panel
// on the right on a wide screen. It traps focus, closes on Escape or the
// scrim, and gives focus back to the card that opened it.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { NeedsApi } from "../core/needs.ts";
import { BoardApi, TRANSITION_WORDS, actionsFor, parseResult, transitionsFor, ago, bearerRefusal, chips, chromeFor, closeFacts, leaseLeft, stateLabel, type CloseFacts, type Gate, type Member, type Thread } from "../core/board.ts";
import { hasToken } from "../core/api.ts";
import { useSession } from "../hooks/useSession.ts";
import { useToolsApi } from "../hooks/useTools.ts";
import { ReviewContext } from "../needs/ReviewContext.tsx";
import { convHash } from "../core/threads.ts";

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
  const wide = typeof window !== "undefined" && window.matchMedia("(min-width: 900px)").matches;
  const off = reduce ? { opacity: 0 } : wide ? { x: 32, opacity: 0 } : { y: 40, opacity: 0 };
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
            initial={off}
            animate={{ x: 0, y: 0, opacity: 1 }}
            exit={off}
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
  const name = (id: string | null | undefined) => (id ? (members.get(id)?.name ?? (members.size ? id.slice(0, 8) : "…")) : "");
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
    if (busy) return;
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
  const holder = th.assignee_id ? members.get(th.assignee_id) : undefined;
  const tone = th.closed_without_review ? "unreviewed" : chrome.lane;
  return (
    <div ref={box} className="sheet-body">
      <div className="grabber" aria-hidden="true" />
      <header className="sheet-head">
        <h2 id="sheet-title">{th.title || th.id.slice(0, 8)}</h2>
        <button className="icon-btn sheet-x" aria-label="Close the task sheet" onClick={onClose} data-autofocus>
          <svg viewBox="0 0 24 24" className="icon" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
        </button>
      </header>

      <dl className="facts">
        <div>
          <dt>State</dt>
          <dd>
            <span className={`state state-${tone}`} title={chrome.hint}>
              <span className="state-dot" aria-hidden="true" />
              <span className="state-text">{stateLabel(th, chrome)}</span>
            </span>
          </dd>
        </div>
        {th.assignee_id && !terminal && (
          <div>
            <dt>Held by</dt>
            <dd className="who">
              <span className={`avatar xs ${holder?.kind === "human" ? "human" : ""}`} aria-hidden="true">{initials(name(th.assignee_id))}</span>
              {name(th.assignee_id)}
              {th.assignment_expires_at ? <span className="muted"> · {leaseLeft(th.assignment_expires_at, now)}</span> : null}
            </dd>
          </div>
        )}
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

      <SheetContext tid={th.id} />
      <a className="ev-open" href={convHash({ kind: "task", id: th.id })}>Open full thread</a>

      <SheetActions th={th} api={api} can={can} claimWhy={claimWhy} busy={busy} act={act} onChanged={onChanged} setNote={setNote} />
      <p className="sheet-note" role="status" aria-live="polite">{note}</p>
    </div>
  );
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/);
  return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase() || "?";
}

type ActionProps = {
  th: Thread;
  api: BoardApi;
  can: { claim: string | null | false; close: boolean };
  claimWhy: string | null;
  busy: "claim" | "close" | null;
  act: (kind: "claim" | "close") => Promise<void>;
  onChanged: () => void;
  setNote: (s: string | null) => void;
};

/**
 * The moves that fit the state up front (Claim when open, Send to review while
 * held, Approve or Send back in review), and the rare or destructive ones
 * (Close, Archive, Record result) behind a menu.
 */
function SheetActions({ th, api, can, claimWhy, busy, act, onChanged, setNote }: ActionProps) {
  const { session, store } = useSession();
  const needs = useMemo(
    () => new NeedsApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  const [working, setWorking] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const [armed, setArmed] = useState(false);
  const [panel, setPanel] = useState<"send-back" | "result" | null>(null);
  const [words, setWords] = useState("");
  const [result, setResult] = useState("");
  const [lease, setLease] = useState("");
  const menuBtn = useRef<HTMLButtonElement>(null);
  const menuBox = useRef<HTMLDivElement>(null);
  const tokenWhy = bearerRefusal(session.credentials);
  const terminal = th.state === "closed" || th.state === "archived";
  const review = th.state === "in_review";
  const moves = transitionsFor(th);
  const disabled = busy !== null || working !== null;

  useEffect(() => {
    if (menu) menuBox.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [menu]);

  const go = async (key: string, fn: () => Promise<unknown>, done: string) => {
    if (disabled) return;
    setWorking(key);
    setNote(null);
    try {
      await fn();
      setNote(done);
      setPanel(null);
      onChanged();
    } catch (e) {
      setNote(e instanceof Error ? e.message : "That did not go through");
    } finally {
      setWorking(null);
      setArmed(false);
    }
  };
  const approve = () =>
    go("approve", async () => {
      const p = await needs.reviewPacket(th.id);
      await needs.review(th.id, "approve", "", p.state === "ready" ? p.packet.evidence_root : null);
    }, "Approved.");
  const sendBack = () => {
    if (!words.trim()) return setNote("Say what should change: the agent reads this.");
    void go("send-back", () => needs.review(th.id, "request_changes", words, null), "Sent back with your note.");
  };
  const record = () => {
    const parsed = parseResult(result);
    if (!parsed.ok) return setNote(parsed.why);
    if (!lease.trim()) return setNote("Paste the claim's lease id: only the claim holder records a result.");
    void go("result", () => api.setResult(th.id, parsed.value, lease), "Result recorded.");
  };
  const closeMenu = (focus = true) => {
    setMenu(false);
    setArmed(false);
    if (focus) menuBtn.current?.focus();
  };
  const onMenuKey = (e: ReactKeyboardEvent) => {
    const items = Array.from(menuBox.current?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? []);
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
    }
  };

  const extra: { key: string; label: string; danger?: boolean; run: () => void }[] = [];
  if (!terminal) extra.push({ key: "result", label: "Record a result…", run: () => { closeMenu(false); setPanel("result"); } });
  if (can.close) extra.push({ key: "close", label: "Close task", danger: true, run: () => { closeMenu(); void act("close"); } });
  if (moves.includes("archive"))
    extra.push({
      key: "archive",
      label: armed ? "Archive? Choose again" : "Archive",
      danger: true,
      run: () => (armed ? (closeMenu(), void go("archive", () => api.transition(th.id, "archive"), "Archived.")) : setArmed(true)),
    });

  return (
    <>
      <div className="sheet-actions">
        {can.claim !== false && (
          <button className="button ink" disabled={disabled || claimWhy !== null} aria-describedby={claimWhy ? "claim-why" : undefined} onClick={() => void act("claim")}>
            {busy === "claim" ? "Claiming…" : "Claim"}
          </button>
        )}
        {review && (
          <>
            <button className="button ink" disabled={disabled} onClick={() => void approve()}>{working === "approve" ? "Approving…" : "Approve"}</button>
            <button className="button steel" disabled={disabled} aria-expanded={panel === "send-back"} onClick={() => setPanel(panel === "send-back" ? null : "send-back")}>Send back</button>
          </>
        )}
        {!review && !terminal && th.assignee_id && moves.includes("start_review") && (
          <button className="button ink" disabled={disabled} onClick={() => void go("review", () => api.transition(th.id, "start_review"), `${TRANSITION_WORDS.start_review}: done.`)}>
            {working === "review" ? "Sending…" : TRANSITION_WORDS.start_review}
          </button>
        )}
        {extra.length > 0 && (
          <div className="sheet-menu" onBlur={(e) => { if (menu && !e.currentTarget.contains(e.relatedTarget as Node | null)) closeMenu(false); }}>
            <button ref={menuBtn} type="button" className="icon-btn sheet-menu-btn" aria-label="More actions" aria-haspopup="menu" aria-expanded={menu} onClick={() => (menu ? closeMenu() : setMenu(true))}>
              <svg viewBox="0 0 24 24" className="icon" aria-hidden="true"><circle cx="5" cy="12" r="1.4" /><circle cx="12" cy="12" r="1.4" /><circle cx="19" cy="12" r="1.4" /></svg>
            </button>
            {menu && (
              <div ref={menuBox} className="menu" role="menu" aria-label="More actions" onKeyDown={onMenuKey}>
                {extra.map((x) => (
                  <button key={x.key} type="button" role="menuitem" className={`menu-item ${x.danger ? "danger" : ""}`} disabled={disabled} onClick={x.run}>
                    {busy === "close" && x.key === "close" ? "Closing…" : x.label}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
      {claimWhy && <p id="claim-why" className="muted small">{claimWhy}</p>}
      {panel === "send-back" && (
        <div className="result-form">
          <label className="field">
            What should change?
            <textarea rows={3} value={words} onChange={(e) => setWords(e.target.value)} placeholder="The agent reads this." />
          </label>
          <button type="button" className="button steel" disabled={disabled} onClick={sendBack}>{working === "send-back" ? "Sending…" : "Send back"}</button>
        </div>
      )}
      {panel === "result" && (
        <div className="result-form">
          <label className="field">
            Result
            <textarea rows={3} value={result} onChange={(e) => setResult(e.target.value)} placeholder="JSON, or a plain summary" />
          </label>
          <label className="field">
            Claim lease id
            <input value={lease} onChange={(e) => setLease(e.target.value)} spellCheck={false} autoCapitalize="off" className="mono" />
          </label>
          <button type="button" className="button steel" disabled={disabled || tokenWhy !== null} onClick={record}>Record result</button>
          {tokenWhy && <p className="muted small">{tokenWhy}</p>}
        </div>
      )}
    </>
  );
}

/** The task's evidence and latest messages, as the review pane shows them. */
function SheetContext({ tid }: { tid: string }) {
  const tools = useToolsApi();
  const [shas, setShas] = useState<{ tid: string; list: string[] } | null>(null);
  useEffect(() => {
    let live = true;
    tools.linked(tid).then(
      (list) => live && setShas({ tid, list }),
      () => live && setShas({ tid, list: [] }),
    );
    return () => {
      live = false;
    };
  }, [tools, tid]);
  return <div className="sheet-context"><ReviewContext tid={tid} shas={shas?.tid === tid ? shas.list : []} /></div>;
}
