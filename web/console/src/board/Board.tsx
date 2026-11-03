// The board, iPhone first: a lane picker over one lane of cards on a phone,
// all four lanes side by side on a wide screen. A card opens the task sheet.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type CSSProperties, type FormEvent } from "react";
import { BoardApi, LANES, ago, cardAriaLabel, chips, leaseLeft, toLanes, type Card, type LaneKey, type Member, type Thread } from "../core/board.ts";
import { Icon } from "../shell/icons.tsx";
import { useBoard } from "../hooks/useBoard.ts";
import { useSession } from "../hooks/useSession.ts";
import { NEW_WORK_HREF, emptyChannelText } from "../core/firstRun.ts";
import { TaskSheet } from "./TaskSheet.tsx";
import { EMPTY_QUERY, filtering, matches, readQuery, writeQuery, type BoardQuery } from "../core/boardQuery.ts";
import { isTyping } from "../shell/commands.ts";
import { useToolsApi } from "../hooks/useTools.ts";

const QUERY_EVENT = "maidan:board-query";
function useQuery(): [BoardQuery, (q: BoardQuery) => void] {
  const hash = useSyncExternalStore(
    (fn) => {
      window.addEventListener("hashchange", fn);
      window.addEventListener(QUERY_EVENT, fn);
      return () => {
        window.removeEventListener("hashchange", fn);
        window.removeEventListener(QUERY_EVENT, fn);
      };
    },
    () => window.location.hash,
  );
  const q = useMemo(() => readQuery(hash), [hash]);
  const set = (next: BoardQuery) => {
    const h = writeQuery(next);
    if (h === window.location.hash) return;
    window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}${h}`);
    window.dispatchEvent(new Event(QUERY_EVENT));
  };
  return [q, set];
}

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
  const [query, setQuery] = useQuery();
  const openId = query.task;
  const setOpenId = (task: string | null) => setQuery({ ...readQuery(window.location.hash), task, channel: task ? data.channelId : null });
  // A task link from another channel switches to that channel once, if the
  // reader can see it, so the drawer can open.
  const linkedChannel = query.task ? query.channel : null;
  useEffect(() => {
    if (!linkedChannel || data.status !== "ready" || data.channelId === linkedChannel) return;
    if (data.channels.some((c) => c.id === linkedChannel)) selectChannel(linkedChannel);
  }, [linkedChannel, data.status, data.channelId, data.channels, selectChannel]);
  const [composing, setComposing] = useState(false);
  const all = useMemo(() => toLanes(data.threads, data.gates), [data.threads, data.gates]);
  const lanes = useMemo(() => {
    const out = { ...all };
    for (const l of LANES) out[l.key] = all[l.key].filter((c) => matches(c, { ...query, state: null }));
    return out;
  }, [all, query]);
  // A task just created opens from the create's answer until the board's own
  // read has it, so a failed refresh does not leave the drawer shut.
  const [created, setCreated] = useState<Thread | null>(null);
  const opened = data.threads.find((t) => t.id === openId) ?? (created?.id === openId ? created : null);
  // The board narrows beside the drawer; keep the chosen card in view.
  const scrolledFor = useRef<string | null>(null);
  const cardLoaded = Boolean(openId && data.threads.some((t) => t.id === openId));
  useEffect(() => {
    if (!openId || !cardLoaded || scrolledFor.current === openId) return;
    // Only the lanes scroll sideways; the page itself never moves.
    const t = window.setTimeout(() => {
      const card = document.querySelector<HTMLElement>(".task.selected");
      const lanes = card?.closest<HTMLElement>(".lanes");
      if (!card || !lanes) return;
      scrolledFor.current = openId;
      const c = card.getBoundingClientRect();
      const l = lanes.getBoundingClientRect();
      if (c.right > l.right) lanes.scrollLeft += c.right - l.right + 12;
      else if (c.left < l.left) lanes.scrollLeft -= l.left - c.left + 12;
    }, 220);
    return () => window.clearTimeout(t);
  }, [openId, cardLoaded]);
  const agents = useMemo(() => {
    const held = new Set(data.threads.map((t) => t.assignee_id).filter(Boolean));
    return [...data.members.values()].filter((m) => held.has(m.id) || (m.kind && m.kind !== "human")).sort((a, b) => a.name.localeCompare(b.name));
  }, [data.threads, data.members]);

  // c starts a new task, as long as nothing else has the keyboard.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "c" || e.metaKey || e.ctrlKey || e.altKey || e.repeat || isTyping(e.target) || document.querySelector("[aria-modal='true']")) return;
      if (!data.channelId) return;
      e.preventDefault();
      setComposing(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [data.channelId]);

  if (session.status === "loading") return <p className="muted">Checking your sign-in…</p>;

  const activeLane = query.state ?? lane;
  const shown = wide ? LANES.filter((l) => !query.state || l.key === query.state) : LANES.filter((l) => l.key === activeLane);
  const total = LANES.reduce((n, l) => n + all[l.key].length, 0);
  const visible = (query.state ? [query.state] : LANES.map((l) => l.key)).reduce((n, k) => n + lanes[k].length, 0);
  return (
    <div className={opened ? "board drawer-open" : "board"}>
      <div className="board-bar">
        <label className="channel-pick">
          <span className="visually-hidden">Channel</span>
          <span className="channel-hash" aria-hidden="true">#</span>
          <select value={data.channelId ?? ""} onChange={(e) => {
              // Choosing a channel closes a linked task, so the link stops pulling the board back.
              setQuery({ ...readQuery(window.location.hash), task: null, channel: null });
              selectChannel(e.target.value);
            }} disabled={!data.channels.length}>
            {data.channels.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
          <Icon name="chevron" />
        </label>
        <p className="board-summary" role="status" aria-live="polite">
          {data.status === "loading" ? "Loading…" : data.error ?? summary(all, data.channels.length)}
        </p>
        <button type="button" className="btn go new-task" onClick={() => setComposing(true)} disabled={!data.channelId} aria-keyshortcuts="c">
          <span>New task</span>
          <kbd aria-hidden="true">C</kbd>
        </button>
      </div>

      {composing && data.channelId && (
        <Composer
          channelId={data.channelId}
          onDone={(made) => {
            setComposing(false);
            if (made?.id) {
              setCreated(made);
              void refresh();
              setOpenId(made.id);
            }
          }}
        />
      )}

      <FilterBar query={query} agents={agents} onChange={(q) => setQuery({ ...q, task: query.task })} total={total} visible={visible} />

      {!wide && (
        <div className="lane-pick" role="tablist" aria-label="Lanes">
          {LANES.map((l) => (
            <button
              key={l.key}
              role="tab"
              id={`tab-${l.key}`}
              aria-selected={activeLane === l.key}
              aria-controls={`lane-${l.key}`}
              tabIndex={activeLane === l.key ? 0 : -1}
              onClick={() => {
                setLane(l.key);
                if (query.state) setQuery({ ...query, state: null });
              }}
              onKeyDown={(e) => {
                const i = LANES.findIndex((x) => x.key === activeLane);
                const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
                if (!step) return;
                e.preventDefault();
                const next = LANES[(i + step + LANES.length) % LANES.length].key;
                setLane(next);
                if (query.state) setQuery({ ...query, state: null });
                document.getElementById(`tab-${next}`)?.focus();
              }}
            >
              {activeLane === l.key && <motion.span className="seg steel" layoutId="lane-seg" transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 500, damping: 40 }} />}
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
        <div className={wide ? `lanes wide${query.state ? " one" : ""}` : "lanes"}>
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
                      <TaskCard card={c} now={now} member={c.thread.assignee_id ? data.members.get(c.thread.assignee_id) : undefined} fresh={data.fresh.has(c.thread.id)} refusal={data.refusals.get(c.thread.id)} selected={openId === c.thread.id} onOpen={() => setOpenId(c.thread.id)} />
                    </motion.li>
                  ))}
                </AnimatePresence>
                {!lanes[l.key].length && data.status === "ready" && <li className="empty">{filtering(query) && all[l.key].length ? <p className="muted small lane-none">Nothing here matches.</p> : <EmptyLane k={l.key} />}</li>}
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

function TaskCard({ card, now, member, fresh, refusal, selected, onOpen }: { card: Card; now: number; member?: Member; fresh: boolean; refusal?: string; selected: boolean; onOpen: () => void }) {
  const th = card.thread;
  const flag = cardFlag(th, card.chrome.lane, refusal);
  const lease = card.chrome.lane === "working" ? leaseView(th.assignment_expires_at, now) : null;
  const tone = th.closed_without_review ? "unreviewed" : card.chrome.lane;
  return (
    <button type="button" className={`task ${fresh ? "fresh" : ""} ${selected ? "selected" : ""}`} aria-current={selected ? "true" : undefined} data-chrome={card.chrome.label} aria-label={`${cardAriaLabel(card)}${refusal !== undefined ? ", close refused" : ""}`} aria-haspopup="dialog" onClick={onOpen}>
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
  const order = ["blocked", "status-blocked", "stalled", "wait-overdue", "wait-waiting", "wait-returned_to_queue"];
  const all = chips(th, lane);
  for (const k of order) {
    const c = all.find((x) => x.kind === k);
    if (c) return { kind: k.startsWith("wait") ? "wait" : k === "status-blocked" ? "blocked" : k, text: c.text, title: c.title ?? c.text };
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

function summary(l: Record<LaneKey, Card[]>, channels: number): string {
  const n = l.open.length + l.working.length + l.review.length + l.done.length;
  if (!n) return emptyChannelText(channels);
  return `${n} task${n === 1 ? "" : "s"} · ${l.working.length} in progress · ${l.review.length} need review`;
}

const EMPTY: Record<LaneKey, { head: string; body: string; action?: { href: string; text: string } }> = {
  open: { head: "All clear", body: "No task is waiting to be claimed. Start one and an agent can pick it up.", action: { href: NEW_WORK_HREF, text: "Start a task" } },
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


function FilterBar({ query, agents, onChange, total, visible }: { query: BoardQuery; agents: Member[]; onChange: (q: BoardQuery) => void; total: number; visible: number }) {
  const on = filtering(query);
  return (
    <div className="board-filters" role="search" aria-label="Filter tasks">
      <label className="filter-q">
        <span className="visually-hidden">Search tasks</span>
        <Icon name="search" />
        <input type="search" placeholder="Search tasks" value={query.q} onChange={(e) => onChange({ ...query, q: e.target.value })} spellCheck={false} />
      </label>
      <label className="filter-pick">
        <span className="visually-hidden">Agent</span>
        <select value={query.agent ?? ""} onChange={(e) => onChange({ ...query, agent: e.target.value || null })}>
          <option value="">Anyone</option>
          {agents.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          {/* A link can name an agent no longer listed; the select still says which filter is on. */}
          {query.agent && !agents.some((m) => m.id === query.agent) ? <option value={query.agent}>{query.agent.slice(0, 8)}</option> : null}
        </select>
        <Icon name="chevron" />
      </label>
      <label className="filter-pick">
        <span className="visually-hidden">State</span>
        <select value={query.state ?? ""} onChange={(e) => onChange({ ...query, state: (e.target.value || null) as BoardQuery["state"] })}>
          <option value="">Any state</option>
          {LANES.map((l) => <option key={l.key} value={l.key}>{l.title}</option>)}
        </select>
        <Icon name="chevron" />
      </label>
      {on && (
        <>
          <span className="filter-count" role="status">{`${visible} of ${total}`}</span>
          <button type="button" className="btn text filter-clear" onClick={() => onChange(EMPTY_QUERY)}>Clear</button>
        </>
      )}
    </div>
  );
}

function Composer({ channelId, onDone }: { channelId: string; onDone: (made: Thread | null) => void }) {
  const tools = useToolsApi();
  const input = useRef<HTMLInputElement>(null);
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => input.current?.focus(), []);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!title.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const made = await tools.createThread(channelId, title);
      onDone(made);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not create the task");
      setBusy(false);
    }
  };
  return (
    <form className="composer" onSubmit={(e) => void submit(e)} onKeyDown={(e) => { if (e.key === "Escape" && !busy) { e.preventDefault(); onDone(null); } }} aria-label="New task">
      <label className="composer-field">
        <span className="visually-hidden">Task title</span>
        <input ref={input} value={title} onChange={(e) => setTitle(e.target.value)} placeholder="What needs doing?" maxLength={200} disabled={busy} />
      </label>
      <button type="submit" className="btn go" disabled={busy || !title.trim()}>{busy ? "Creating…" : "Create"}</button>
      <button type="button" className="btn text" onClick={() => onDone(null)} disabled={busy}>Cancel</button>
      {error && <p className="composer-error" role="alert">{error}</p>}
    </form>
  );
}
