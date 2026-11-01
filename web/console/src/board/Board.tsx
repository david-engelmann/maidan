// The board, iPhone first: a lane picker over one lane of cards on a phone,
// all four lanes side by side on a wide screen. A card opens the task sheet.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { ANIMATE_UP_TO, BoardApi, LANE_PAGE, LANES, windowed, boardHealth, byAgent, presenceStrip, cardAriaLabel, cardWhen, chips, healthMatch, staleBanner, toLanes, type Card, type HealthKey, type LaneKey, type Member } from "../core/board.ts";
import { liveStatus } from "../core/bus.ts";
import { useBoard } from "../hooks/useBoard.ts";
import { usePresence } from "../hooks/usePeople.ts";
import { useSession } from "../hooks/useSession.ts";
import { TaskSheet } from "./TaskSheet.tsx";

const WIDE = "(min-width: 900px)";
const VIEW_KEY = "maidan_console_board_view";
const PRESENCE_SHOWN = 8;
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
  const presence = usePresence();
  const here = useMemo(() => presenceStrip(data.members, presence, data.threads), [data.members, presence, data.threads]);
  const live = useSyncExternalStore(liveStatus.subscribe, liveStatus.getSnapshot);
  const stale = staleBanner(live, data.loadedAt, data.failedSince, now);
  const [focus, setFocus] = useState<HealthKey | null>(null);
  const health = useMemo(() => boardHealth(data.threads, now), [data.threads, now]);
  // A filter whose signal has cleared falls away on its own.
  const active = focus && health.some((h) => h.key === focus) ? focus : null;
  const shownThreads = useMemo(
    () => (active ? data.threads.filter((t) => healthMatch(t, active, now)) : data.threads),
    [data.threads, active, now],
  );
  const lanes = useMemo(() => toLanes(shownThreads, data.gates), [shownThreads, data.gates]);
  // How many cards each lane or group renders; a channel switch starts over.
  const [limits, setLimits] = useState<Record<string, number>>({});
  const [limitsFor, setLimitsFor] = useState(data.channelId);
  if (limitsFor !== data.channelId) {
    setLimitsFor(data.channelId);
    setLimits({});
  }
  const limitOf = (k: string) => limits[k] ?? LANE_PAGE;
  const more = (k: string) => setLimits((m) => ({ ...m, [k]: (m[k] ?? LANE_PAGE) + LANE_PAGE }));
  const [view, setViewState] = useState<"lanes" | "agents">(() => (localStorage.getItem(VIEW_KEY) === "agents" ? "agents" : "lanes"));
  const setView = (v: "lanes" | "agents") => {
    localStorage.setItem(VIEW_KEY, v);
    setViewState(v);
  };
  const groups = useMemo(() => (view === "agents" ? byAgent(shownThreads, data.gates, data.members) : []), [view, shownThreads, data.gates, data.members]);
  const opened = data.threads.find((t) => t.id === openId) ?? null;

  if (session.status === "loading") return <p className="muted">Checking your sign-in…</p>;

  const shown = wide ? LANES : LANES.filter((l) => l.key === lane);
  return (
    <div className="board">
      <div className="board-bar">
        <label className="channel-pick">
          <span className="mono">Channel</span>
          <select value={data.channelId ?? ""} onChange={(e) => selectChannel(e.target.value)} disabled={!data.channels.length}>
            {data.channels.map((c) => (
              <option key={c.id} value={c.id}>#{c.name}</option>
            ))}
          </select>
        </label>
        <div className="view-pick" role="group" aria-label="Board view">
          <button type="button" className="chip-btn" aria-pressed={view === "lanes"} onClick={() => setView("lanes")}>Lanes</button>
          <button type="button" className="chip-btn" aria-pressed={view === "agents"} onClick={() => setView("agents")}>By agent</button>
        </div>
        <p className="board-summary" role="status" aria-live="polite">
          {data.status === "loading" ? "Loading…" : data.error ?? summary(lanes)}
        </p>
      </div>

      {stale && (
        <div className="stale-banner" role="status">
          <span>{stale}</span>
          <button type="button" className="chip-btn" onClick={() => void refresh()}>Refresh now</button>
        </div>
      )}

      {here.length > 0 && (
        <div className="presence-strip" role="group" aria-label="Who is here">
          <ul>
            {here.slice(0, PRESENCE_SHOWN).map((p) => (
              <li key={p.id} className={`here here-${p.status}`} title={`${p.name}: ${p.status}${p.holding ? `, holding ${p.holding} task${p.holding === 1 ? "" : "s"}` : ", free"}`}>
                <span className="dot" aria-hidden="true" />
                <span>{p.name}</span>
                <span className="sr-only">{`, ${p.status}${p.holding ? `, holding ${p.holding}` : ", free"}`}</span>
                {p.holding > 0 && <span className="count" aria-hidden="true">{p.holding}</span>}
              </li>
            ))}
          </ul>
          {here.length > PRESENCE_SHOWN && <a href="#/people" className="muted small">{`+${here.length - PRESENCE_SHOWN} more`}</a>}
        </div>
      )}

      {health.length > 0 && (
        <div className="health" role="group" aria-label="Board health">
          {health.map((h) => (
            <button
              key={h.key}
              type="button"
              className={`health-chip health-${h.key}`}
              aria-pressed={active === h.key}
              title={`${h.title}. ${active === h.key ? "Show every task" : "Show only these"}`}
              onClick={() => setFocus(active === h.key ? null : h.key)}
            >
              {h.text}
            </button>
          ))}
          {active && (
            <button type="button" className="health-chip health-clear" onClick={() => setFocus(null)}>Show all</button>
          )}
        </div>
      )}

      {!wide && view === "lanes" && (
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
              <span className="sr-only">{`${l.title}, ${lanes[l.key].length}`}</span>
            </button>
          ))}
        </div>
      )}

      {data.status === "error" && !data.threads.length ? (
        <div className="card">
          <p>{data.error}</p>
          <button className="button steel" onClick={() => void refresh()}>Try again</button>
        </div>
      ) : view === "agents" ? (
        <div className="agents">
          {groups.map((g) => (
            <section key={g.id ?? "unclaimed"} className="agent-group" aria-labelledby={`ag-${g.id ?? "unclaimed"}`}>
              <h2 id={`ag-${g.id ?? "unclaimed"}`} className="lane-head">
                <span>{g.name}</span>
                <span className="count">{g.cards.length}</span>
              </h2>
              <ul className="cards">
                {windowed(g.cards, limitOf(`ag-${g.id ?? ""}`)).shown.map((c) => (
                  <li key={c.thread.id}>
                    <TaskCard card={c} now={now} fresh={data.fresh.has(c.thread.id)} refusal={data.refusals.get(c.thread.id)} onOpen={() => setOpenId(c.thread.id)} />
                  </li>
                ))}
              </ul>
              <MoreCards hidden={windowed(g.cards, limitOf(`ag-${g.id ?? ""}`)).hidden} onMore={() => more(`ag-${g.id ?? ""}`)} />
            </section>
          ))}
          {!groups.length && data.status === "ready" && <p className="muted">No live tasks in this channel.</p>}
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
              <ul className="cards">
                <AnimatePresence initial={false}>
                  {windowed(lanes[l.key], limitOf(l.key)).shown.map((c) => (
                    <motion.li
                      key={c.thread.id}
                      layout={!reduce && lanes[l.key].length <= ANIMATE_UP_TO}
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
              <MoreCards hidden={windowed(lanes[l.key], limitOf(l.key)).hidden} onMore={() => more(l.key)} />
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
  return (
    <button type="button" className={`task ${fresh ? "fresh" : ""}`} data-chrome={card.chrome.label} aria-label={`${cardAriaLabel(card)}${refusal !== undefined ? ", close refused" : ""}`} aria-haspopup="dialog" onClick={onOpen}>
      <span className="task-title">{card.title}</span>
      <span className="task-foot">
        <span className={`state state-${card.chrome.lane}`}>{card.label}</span>
        {chips(th, card.chrome.lane).map((ch) => (
          <span key={ch.kind} className={`chip chip-${ch.kind}`} title={ch.title}>{ch.text}</span>
        ))}
        {refusal !== undefined && <span className="chip chip-refused" title={refusal || "The server refused this close"}>Close refused</span>}
        {member && <span className="who">{member.name}</span>}
        <span className="when">{cardWhen(card, now)}</span>
      </span>
    </button>
  );
}

/** "Show N more" under a long lane; it also loads more on its own as it scrolls into view. */
function MoreCards({ hidden, onMore }: { hidden: number; onMore: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const latest = useRef(onMore);
  useEffect(() => {
    latest.current = onMore;
  });
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) latest.current();
    }, { rootMargin: "400px" });
    io.observe(el);
    return () => io.disconnect();
  }, [hidden]);
  if (!hidden) return null;
  return (
    <button ref={ref} type="button" className="chip-btn more-cards" onClick={onMore}>
      {`Show ${Math.min(hidden, LANE_PAGE)} more of ${hidden}`}
    </button>
  );
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

