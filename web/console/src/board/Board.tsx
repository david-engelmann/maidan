// The board, iPhone first: a lane picker over one lane of cards on a phone,
// all four lanes side by side on a wide screen. A card opens the task sheet.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useState, useSyncExternalStore, type CSSProperties } from "react";
import { BoardApi, LANES, ago, cardAriaLabel, chips, leaseLeft, toLanes, type Card, type LaneKey, type Member, type Thread } from "../core/board.ts";
import { Icon } from "../shell/icons.tsx";
import { useBoard } from "../hooks/useBoard.ts";
import { useSession } from "../hooks/useSession.ts";
import { TaskSheet } from "./TaskSheet.tsx";

const WIDE = "(min-width: 900px)";
function useWide(): boolean {
  return useSyncExternalStore(
    (fn) => {
      const mq = window.matchMedia(WIDE);
      mq.addEventListener("change", fn);
      return () => mq.removeEventListener("change", fn);
    },
    () => window.matchMedia(WIDE).matches,
  );
}

/** A clock that ticks each 30 s, so "5m ago" and leases stay true. */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(t);
  }, []);
  return now;
}

export function Board() {
  const { session, store } = useSession();
  // The api reads the live session each call, so a sign-in change needs no new api.
  const api = useMemo(
    () =>
      new BoardApi(
        (url, init) => fetch(url, init),
        () => store.getSnapshot().credentials,
        () => store.getSnapshot().mode,
      ),
    [store],
  );
  const { data, selectChannel, refresh } = useBoard(session, api);
  const wide = useWide();
  const now = useNow();
  const reduce = Boolean(useReducedMotion());
  const [lane, setLane] = useState<LaneKey>("open");
  const [openId, setOpenId] = useState<string | null>(null);
  const lanes = useMemo(() => toLanes(data.threads, data.gates), [data.threads, data.gates]);
  const opened = data.threads.find((t) => t.id === openId) ?? null;

  if (session.status === "loading") return <p className="muted">Checking your sign-in…</p>;

  const shown = wide ? LANES : LANES.filter((l) => l.key === lane);
  return (
    <div className="board">
      <div className="board-bar">
        <label className="channel-pick">
          <span className="visually-hidden">Channel</span>
          <span className="channel-hash" aria-hidden="true">#</span>
          <select value={data.channelId ?? ""} onChange={(e) => selectChannel(e.target.value)} disabled={!data.channels.length}>
            {data.channels.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
          <Icon name="chevron" />
        </label>
        <p className="board-summary" role="status" aria-live="polite">
          {data.status === "loading" ? "Loading…" : data.error ?? summary(lanes)}
        </p>
      </div>

      {!wide && (
        <div className="lane-pick" role="tablist" aria-label="Lanes">
          {LANES.map((l) => (
            <button
              key={l.key}
              role="tab"
              id={`tab-${l.key}`}
              aria-selected={lane === l.key}
              aria-controls={`lane-${l.key}`}
              tabIndex={lane === l.key ? 0 : -1}
              onClick={() => setLane(l.key)}
              onKeyDown={(e) => {
                const i = LANES.findIndex((x) => x.key === lane);
                const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
                if (!step) return;
                e.preventDefault();
                const next = LANES[(i + step + LANES.length) % LANES.length].key;
                setLane(next);
                document.getElementById(`tab-${next}`)?.focus();
              }}
            >
              {lane === l.key && <motion.span className="seg steel" layoutId="lane-seg" transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 500, damping: 40 }} />}
              <span className="seg-label" aria-hidden="true">{l.short}</span>
              <span className="count" aria-hidden="true">{lanes[l.key].length}</span>
              <span className="visually-hidden">{`${l.title}, ${lanes[l.key].length}`}</span>
            </button>
          ))}
        </div>
      )}

      {data.status === "error" && !data.threads.length ? (
        <div className="card">
          <p>{data.error}</p>
          <button className="button steel" onClick={() => void refresh()}>Try again</button>
        </div>
      ) : (
        <div className={wide ? "lanes wide" : "lanes"}>
          {shown.map((l) => (
            <section
              key={l.key}
              id={`lane-${l.key}`}
              className="lane"
              role={wide ? "region" : "tabpanel"}
              aria-labelledby={wide ? `h-${l.key}` : `tab-${l.key}`}
            >
              {wide && (
                <h2 id={`h-${l.key}`} className="lane-head">
                  <span>{l.title}</span>
                  <span className="count">{lanes[l.key].length}</span>
                </h2>
              )}
              {data.status === "loading" && !data.threads.length ? (
                <div className="cards" aria-hidden="true">
                  {[0, 1].map((i) => (
                    <span key={i} className="task task-sk">
                      <span className="skeleton sk-title" />
                      <span className="task-meta"><span className="skeleton sk-chip" /><span className="skeleton sk-time" /></span>
                    </span>
                  ))}
                </div>
              ) : null}
              <ul className="cards">
                <AnimatePresence initial={false}>
                  {lanes[l.key].map((c) => (
                    <motion.li
                      key={c.thread.id}
                      layout={!reduce}
                      initial={reduce ? false : { opacity: 0, y: 6 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: 0.97 }}
                      transition={{ duration: 0.18 }}
                    >
                      <TaskCard card={c} now={now} member={c.thread.assignee_id ? data.members.get(c.thread.assignee_id) : undefined} fresh={data.fresh.has(c.thread.id)} refusal={data.refusals.get(c.thread.id)} onOpen={() => setOpenId(c.thread.id)} />
                    </motion.li>
                  ))}
                </AnimatePresence>
                {!lanes[l.key].length && data.status === "ready" && <li className="empty"><EmptyLane k={l.key} /></li>}
              </ul>
            </section>
          ))}
        </div>
      )}

      <TaskSheet
        thread={opened}
        gate={opened ? data.gates[opened.id] : undefined}
        members={data.members}
        api={api}
        now={now}
        onClose={() => setOpenId(null)}
        onChanged={() => void refresh()}
      />
    </div>
  );
}

function TaskCard({ card, now, member, fresh, refusal, onOpen }: { card: Card; now: number; member?: Member; fresh: boolean; refusal?: string; onOpen: () => void }) {
  const th = card.thread;
  const flag = cardFlag(th, card.chrome.lane, refusal);
  const lease = card.chrome.lane === "working" ? leaseView(th.assignment_expires_at, now) : null;
  const tone = th.closed_without_review ? "unreviewed" : card.chrome.lane;
  return (
    <button type="button" className={`task ${fresh ? "fresh" : ""}`} data-chrome={card.chrome.label} aria-label={`${cardAriaLabel(card)}${refusal !== undefined ? ", close refused" : ""}`} aria-haspopup="dialog" onClick={onOpen}>
      <span className="task-title">{card.title}</span>
      <span className="task-meta">
        <span className={`state state-${tone}`} title={th.closed_without_review ? "Closed without review" : th.status?.note || undefined}>
          <span className="state-dot" aria-hidden="true" />
          <span className="state-text">{th.closed_without_review ? "Unreviewed" : card.label}</span>
        </span>
        {flag ? <span className={`task-flag flag-${flag.kind}`} title={flag.title}>{flag.text}</span> : null}
        <span className="task-end">
          {member ? (
            <span className={`avatar xs ${member.kind === "human" ? "human" : ""}`} title={member.name}>{initials(member.name)}</span>
          ) : card.chrome.lane !== "done" ? (
            <span className="avatar xs unclaimed" title="Unclaimed" />
          ) : null}
          <span className="when">{lease?.urgent ? lease.text : ago(th.updated_at, now)}</span>
        </span>
      </span>
      {lease ? (
        <span className={`lease ${lease.urgent ? "urgent" : ""}`} title={lease.text} aria-hidden="true">
          <span className="lease-fill" style={{ "--left": lease.frac } as CSSProperties} />
        </span>
      ) : null}
    </button>
  );
}

/** The one flag worth a card's space: a refusal, a block, a stall, or a wait. */
function cardFlag(th: Thread, lane: LaneKey, refusal?: string): { kind: string; text: string; title?: string } | null {
  if (refusal !== undefined) return { kind: "refused", text: "Close refused", title: refusal || "The server refused this close" };
  const order = ["blocked", "stalled", "wait-overdue", "wait-waiting", "wait-returned_to_queue"];
  const all = chips(th, lane);
  for (const k of order) {
    const c = all.find((x) => x.kind === k);
    if (c) return { kind: k.startsWith("wait") ? "wait" : k, text: c.text, title: c.title ?? c.text };
  }
  return null;
}

const LEASE_WINDOW = 30 * 60;

/** A claim's lease as a bar: shown only while worked, loud only when short. */
function leaseView(ts: string | null | undefined, now: number): { frac: number; text: string; urgent: boolean } | null {
  if (!ts) return null;
  const at = Date.parse(ts);
  if (Number.isNaN(at)) return null;
  const secs = Math.round((at - now) / 1000);
  return { frac: Math.max(0, Math.min(1, secs / LEASE_WINDOW)), text: leaseLeft(ts, now), urgent: secs < 300 };
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/);
  return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase() || "?";
}

function summary(l: Record<LaneKey, Card[]>): string {
  const n = l.open.length + l.working.length + l.review.length + l.done.length;
  if (!n) return "No tasks in this channel yet.";
  return `${n} task${n === 1 ? "" : "s"} · ${l.working.length} in progress · ${l.review.length} need review`;
}

const EMPTY: Record<LaneKey, { head: string; body: string; action?: { href: string; text: string } }> = {
  open: { head: "All clear", body: "No task is waiting to be claimed. Start one in a thread and an agent can pick it up.", action: { href: "#/threads", text: "Start a task" } },
  working: { head: "Nobody is mid-task", body: "Claimed tasks show here while an agent works on them.", action: { href: "#/tools/connect", text: "Connect an agent" } },
  review: { head: "Nothing to review", body: "Results that need your eyes land here. You are caught up." },
  done: { head: "Nothing closed yet", body: "Finished tasks collect here once they are closed." },
};

function EmptyLane({ k }: { k: LaneKey }) {
  const e = EMPTY[k];
  return (
    <div className="empty-state">
      <strong>{e.head}</strong>
      <p className="muted">{e.body}</p>
      {e.action ? <a className="button ink" href={e.action.href}>{e.action.text}</a> : null}
    </div>
  );
}

