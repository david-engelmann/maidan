// The board, iPhone first: a lane picker over one lane of cards on a phone,
// all four lanes side by side on a wide screen. A card opens the task sheet.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type CSSProperties, type FormEvent } from "react";
import { ANIMATE_UP_TO, BoardApi, LANE_PAGE, LANES, windowed, boardHealth, byAgent, presenceStrip, presenceWords, ago, cardAriaLabel, chips, healthMatch, leaseLeft, staleBanner, badgeTargets, shownBadgeTargets, limitForOpen, hasVotes, voteWords, type VoteBadge, toLanes, type Card, type HealthKey, type LaneKey, type Member, type Thread } from "../core/board.ts";
import { useCardVotes } from "../hooks/useCardVotes.ts";
import { liveStatus } from "../core/bus.ts";
import { offlineBanner, reach } from "../core/api.ts";
import { Icon } from "../shell/icons.tsx";
import { useBoard } from "../hooks/useBoard.ts";
import { usePresence } from "../hooks/usePeople.ts";
import { useSession } from "../hooks/useSession.ts";
import { NEW_WORK_HREF, dismissFirstRun, emptyChannelText, firstRunDismissed, guideTasks, showFirstRun } from "../core/firstRun.ts";
import { PeopleApi } from "../core/people.ts";
import { FirstRun } from "./FirstRun.tsx";
import { TaskSheet } from "./TaskSheet.tsx";
import { EMPTY_QUERY, filtering, matches, readQuery, writeQuery, type BoardQuery } from "../core/boardQuery.ts";
import { isTyping } from "../shell/commands.ts";
import { useToolsApi } from "../hooks/useTools.ts";
import { LoadError } from "../shell/LoadError";
import { EmptyState } from "../shell/EmptyState";
import { ChannelPick } from "../shell/ChannelPick.tsx";

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
const VIEW_KEY = "maidan_console_board_view";
const PRESENCE_SHOWN = 4;
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
  // A link that names a channel (?channel=<id>, with or without a task)
  // switches to it once the board can see it, so a shared task and a palette
  // jump land on that channel.
  const linkedChannel = query.channel;
  useEffect(() => {
    if (!linkedChannel || data.status !== "ready" || data.channelId === linkedChannel) return;
    if (data.channels.some((c) => c.id === linkedChannel)) selectChannel(linkedChannel);
  }, [linkedChannel, data.status, data.channelId, data.channels, selectChannel]);
  const [composing, setComposing] = useState(false);
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const [putAway, setPutAway] = useState<string | null>(null);
  const guide = showFirstRun(data.status, guideTasks(data.threads), putAway === workspace || firstRunDismissed(workspace));
  const people = useMemo(
    () =>
      new PeopleApi(
        (url, init) => fetch(url, init),
        () => store.getSnapshot().credentials,
        () => store.getSnapshot().mode,
      ),
    [store],
  );
  const presence = usePresence();
  const live = useSyncExternalStore(liveStatus.subscribe, liveStatus.getSnapshot);
  const here = useMemo(() => presenceStrip(data.members, presence, data.threads, live), [data.members, presence, data.threads, live]);
  // While the shell's offline line is up it says this for every place, so
  // the board does not say it twice.
  const offline = offlineBanner(useSyncExternalStore(reach.subscribe, reach.getSnapshot)) !== null;
  const stale = offline ? null : staleBanner(live, data.loadedAt, data.failedSince, now);
  const [focus, setFocus] = useState<HealthKey | null>(null);
  const health = useMemo(() => boardHealth(data.threads, now, data.gates), [data.threads, now, data.gates]);
  // A filter whose signal has cleared falls away for good, so it can't come back on its own.
  const active = focus && health.some((h) => h.key === focus) ? focus : null;
  if (focus && !active) setFocus(null);
  const shownThreads = useMemo(
    () => (active ? data.threads.filter((t) => healthMatch(t, active, now, data.gates)) : data.threads),
    [data.threads, active, now, data.gates],
  );
  const all = useMemo(() => toLanes(data.threads, data.gates), [data.threads, data.gates]);
  const lanes = useMemo(() => {
    const out = toLanes(shownThreads, data.gates);
    for (const l of LANES) out[l.key] = out[l.key].filter((c) => matches(c, { ...query, state: null }));
    return out;
  }, [shownThreads, data.gates, query]);
  // How many cards each lane or group renders; a channel switch starts over.
  const [limits, setLimits] = useState<Record<string, number>>({});
  const [limitsFor, setLimitsFor] = useState(data.channelId);
  if (limitsFor !== data.channelId) {
    setLimitsFor(data.channelId);
    setLimits({});
  }
  const limitOf = (k: string, cards: Card[]) => limitForOpen(cards, limits[k] ?? LANE_PAGE, openId);
  const more = (k: string) => setLimits((m) => ({ ...m, [k]: (m[k] ?? LANE_PAGE) + LANE_PAGE }));
  const [view, setViewState] = useState<"lanes" | "agents">(() => (localStorage.getItem(VIEW_KEY) === "agents" ? "agents" : "lanes"));
  const setView = (v: "lanes" | "agents") => {
    setViewState(v);
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {
      // Storage full or blocked: the view still changes, it just isn't remembered.
    }
  };
  const groups = useMemo(() => {
    if (view !== "agents") return [];
    // The State filter narrows By agent the same way it narrows the lanes.
    const keys = query.state ? [query.state] : LANES.map((l) => l.key);
    const ids = new Set(keys.flatMap((k) => lanes[k].map((c) => c.thread.id)));
    return byAgent(shownThreads.filter((t) => ids.has(t.id)), data.gates, data.members);
  }, [view, lanes, shownThreads, data.gates, data.members, query.state]);
  // Badges follow the cards on screen, including a "Show more" window and a deep-linked card past the first page.
  const voteIds = useMemo(() => {
    if (view === "agents") {
      const shown = groups.flatMap((g) => windowed(g.cards, limitForOpen(g.cards, limits[`ag-${g.id ?? ""}`] ?? LANE_PAGE, openId)).shown);
      return shownBadgeTargets(shown);
    }
    return badgeTargets(lanes, {
      review: limitForOpen(lanes.review, limits.review ?? LANE_PAGE, openId),
      working: limitForOpen(lanes.working, limits.working ?? LANE_PAGE, openId),
    });
  }, [view, groups, lanes, limits, openId]);
  const votes = useCardVotes(api, voteIds);
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
      // A deep-linked card past the first page sits below the fold.
      if (c.top < 0 || c.bottom > window.innerHeight) card.scrollIntoView({ block: "nearest", inline: "nearest" });
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
      {/* One toolbar row: where, how, which, who and the next action. Filters,
          health and presence are chips in it, not rows of their own, so the
          first task sits right under it. */}
      <div className="board-bar" role="group" aria-label="Board controls">
        <ChannelPick
          channels={data.channels}
          channelId={data.channelId}
          onChange={(id) => {
            // Choosing a channel closes a linked task, so the link stops pulling the board back.
            setQuery({ ...readQuery(window.location.hash), task: null, channel: null });
            selectChannel(id);
          }}
        />
        <div className="view-pick" role="group" aria-label="Board view">
          <button type="button" className="chip-btn" aria-pressed={view === "lanes"} onClick={() => setView("lanes")}>Lanes</button>
          <button type="button" className="chip-btn" aria-pressed={view === "agents"} onClick={() => setView("agents")}>By agent</button>
        </div>
        <FilterBar query={query} agents={agents} onChange={(q) => setQuery({ ...q, task: query.task })} total={total} visible={visible} />
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
        {here.length > 0 && (
          <div className="presence-strip" role="group" aria-label="Who is here">
            <ul>
              {here.slice(0, PRESENCE_SHOWN).map((p) => (
                <li key={p.id} className={`here here-${p.status}`} title={`${p.name}: ${presenceWords(p)}`}>
                  <span className="dot" aria-hidden="true" />
                  {/* The status rides inside the name's span: as a flex item of its own it got the chip's gap, a space before the comma. */}
                  <span>{p.name}<span className="visually-hidden">{`, ${presenceWords(p)}`}</span></span>
                  {p.holding > 0 && <span className="count" title="Tasks held in this channel" aria-hidden="true">{p.holding}</span>}
                </li>
              ))}
            </ul>
            {here.length > PRESENCE_SHOWN && <a href="#/people" className="muted small">{`+${here.length - PRESENCE_SHOWN} more`}</a>}
          </div>
        )}
        <p className="board-summary" role="status" aria-live="polite">
          {data.status === "loading" ? "Loading…" : data.status === "error" && !data.threads.length ? "" : data.error ?? summary(all, data.channels.length)}
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

      {guide && (
        <FirstRun
          base={session.credentials.base || window.location.origin}
          workspace={workspace}
          api={workspace ? people : null}
          channel={data.channels.find((c) => c.id === data.channelId)?.name ?? null}
          channelId={data.channelId}
          onNewTask={data.channelId ? () => setComposing(true) : null}
          onDismiss={() => {
            setPutAway(workspace);
            dismissFirstRun(workspace);
          }}
        />
      )}

      {stale && (
        <div className="stale-banner" role="status">
          <span>{stale}</span>
          <button type="button" className="chip-btn" onClick={() => void refresh()}>Refresh now</button>
        </div>
      )}

      {!wide && view === "lanes" && (
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
        <LoadError message={data.error ?? "Could not load the board."} onRetry={() => void refresh()} />
      ) : view === "agents" ? (
        <div className="agents">
          {groups.map((g) => (
            <section key={g.id ?? "unclaimed"} className="agent-group" aria-labelledby={`ag-${g.id ?? "unclaimed"}`}>
              <h2 id={`ag-${g.id ?? "unclaimed"}`} className="lane-head">
                <span>{g.name}</span>
                <span className="count">{g.cards.length}</span>
              </h2>
              <ul className="cards">
                {windowed(g.cards, limitOf(`ag-${g.id ?? ""}`, g.cards)).shown.map((c) => (
                  <li key={c.thread.id}>
                    <TaskCard card={c} now={now} fresh={data.fresh.has(c.thread.id)} refusal={data.refusals.get(c.thread.id)} member={c.thread.assignee_id ? data.members.get(c.thread.assignee_id) : undefined} votes={votes.get(c.thread.id)} selected={openId === c.thread.id} onOpen={() => setOpenId(c.thread.id)} />
                  </li>
                ))}
              </ul>
              <MoreCards hidden={windowed(g.cards, limitOf(`ag-${g.id ?? ""}`, g.cards)).hidden} onMore={() => more(`ag-${g.id ?? ""}`)} />
            </section>
          ))}
          {!groups.length && data.status === "ready" && <p className="muted">{(filtering(query) || active) && LANES.some((l) => all[l.key].length) ? "Nothing here matches." : "No live tasks in this channel."}</p>}
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
                  {windowed(lanes[l.key], limitOf(l.key, lanes[l.key])).shown.map((c) => (
                    <motion.li
                      key={c.thread.id}
                      layout={!reduce && lanes[l.key].length <= ANIMATE_UP_TO}
                      initial={reduce ? false : { opacity: 0, y: 6 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: 0.97 }}
                      transition={reduce ? { duration: 0 } : { duration: 0.18 }}
                    >
                      <TaskCard card={c} now={now} member={c.thread.assignee_id ? data.members.get(c.thread.assignee_id) : undefined} fresh={data.fresh.has(c.thread.id)} refusal={data.refusals.get(c.thread.id)} votes={votes.get(c.thread.id)} selected={openId === c.thread.id} onOpen={() => setOpenId(c.thread.id)} />
                    </motion.li>
                  ))}
                </AnimatePresence>
                {!lanes[l.key].length && data.status === "ready" && <li className="empty">{(filtering(query) || active) && all[l.key].length ? <p className="muted small lane-none">Nothing here matches.</p> : <EmptyLane k={l.key} />}</li>}
              </ul>
              <MoreCards hidden={windowed(lanes[l.key], limitOf(l.key, lanes[l.key])).hidden} onMore={() => more(l.key)} />
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

function TaskCard({ card, now, member, fresh, refusal, votes, selected, onOpen }: { card: Card; now: number; member?: Member; fresh: boolean; refusal?: string; votes?: VoteBadge; selected: boolean; onOpen: () => void }) {
  const th = card.thread;
  const flag = cardFlag(th, card.chrome.lane, refusal);
  const lease = card.chrome.lane === "working" ? leaseView(th.assignment_expires_at, now) : null;
  const tone = th.closed_without_review ? "unreviewed" : card.chrome.lane;
  return (
    <button type="button" className={`task ${fresh ? "fresh" : ""} ${selected ? "selected" : ""}`} aria-current={selected ? "true" : undefined} data-chrome={card.chrome.label} aria-label={`${cardAriaLabel(card)}${refusal !== undefined ? ", close refused" : ""}${hasVotes(votes) ? `, votes: ${voteWords(votes)}` : ""}`} aria-haspopup="dialog" onClick={onOpen}>
      <span className="task-title">{card.title}</span>
      <span className="task-meta">
        <span className={`state state-${tone}`} title={th.closed_without_review ? "Closed without review" : th.status?.note || undefined}>
          <span className="state-dot" aria-hidden="true" />
          <span className="state-text">{th.closed_without_review ? "Unreviewed" : card.label}</span>
        </span>
        {flag ? <span className={`task-flag flag-${flag.kind}`} title={flag.title}>{flag.text}</span> : null}
        {hasVotes(votes) && (
          <span className="chip chip-votes" title={voteWords(votes)} aria-hidden="true">
            {votes.approve ? <span className="vote-n approve">✓{votes.approve}</span> : null}
            {votes.request_changes ? <span className="vote-n changes">✗{votes.request_changes}</span> : null}
            {votes.ack ? <span className="vote-n ack">ack {votes.ack}</span> : null}
          </span>
        )}
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
    <EmptyState title={e.head} body={e.body} action={e.action ? <a className="button ink" href={e.action.href}>{e.action.text}</a> : undefined} />
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
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- Escape closes the composer from any of its fields.
    <form className="task-composer" onSubmit={(e) => void submit(e)} onKeyDown={(e) => { if (e.key === "Escape" && !busy) { e.preventDefault(); onDone(null); } }} aria-label="New task">
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
