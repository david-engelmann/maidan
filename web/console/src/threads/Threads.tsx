// Threads and DMs, iPhone first: a list of conversations, and one open
// conversation that fills the screen with its composer above the keyboard.
// On a wide screen the list and the conversation sit side by side.
import { MessageBody } from "./MessageBody.tsx";
import { takeCompose } from "../shell/commands.ts";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Icon } from "../shell/icons.tsx";
import { BoardApi, chromeFor, stateLabel, type Gate, type Member, type Thread } from "../core/board.ts";
import {
  QUICK_REACTIONS,
  REACTION_NAMES,
  ThreadsApi,
  ThreadsError,
  atBottom,
  byRecent,
  clockTime,
  convHash,
  dmConversation,
  groupConversation,
  groupRefusal,
  editRefusal,
  keyboardInset,
  nameOf,
  pinnedMessages,
  onNewMessages,
  parseConv,
  pastedFiles,
  sameRef,
  slashResult,
  startsGroup,
  dayKey,
  dayLabel,
  isUnread,
  markOpened,
  type SeenState,
  taskConversation,
  visibleMessages,
  type ConvRef,
  type Conversation,
  type Edit,
  type TaskFacts,
  evidenceLine,
  evidenceRefusal,
  safeHref,
  withRecords,
  EVIDENCE_KINDS,
  EVIDENCE_KIND_WORD,
  type EvidenceDraft,
  type EvidenceKind,
  type Message,
  type Reaction,
} from "../core/threads.ts";
import { say, sayError } from "../core/notices.ts";
import { useBoard } from "../hooks/useBoard.ts";
import { useSession } from "../hooks/useSession.ts";
import { NEW_WORK_HREF, emptyChannelText } from "../core/firstRun.ts";
import { useConversation, useDirect, useVisualViewport } from "../hooks/useThreads.ts";

type Tab = "tasks" | "direct";

function subscribeHash(fn: () => void): () => void {
  window.addEventListener("hashchange", fn);
  return () => window.removeEventListener("hashchange", fn);
}

function useOpenRef(): ConvRef | null {
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);
  return useMemo(() => parseConv(hash), [hash]);
}

function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

const SEEN_PREFIX = "maidan:seen:";

function loadSeen(key: string | null): SeenState {
  const fresh = { baseline: new Date().toISOString(), opened: {} };
  if (!key) return fresh;
  try {
    const raw = window.localStorage.getItem(SEEN_PREFIX + key);
    const v = raw ? (JSON.parse(raw) as Partial<SeenState>) : null;
    if (v && typeof v.baseline === "string" && v.opened && typeof v.opened === "object") return { baseline: v.baseline, opened: v.opened };
  } catch {
    // A private window or a full disk: unread marks last for this visit only.
  }
  try {
    window.localStorage.setItem(SEEN_PREFIX + key, JSON.stringify(fresh));
  } catch {
    // As above.
  }
  return fresh;
}

function saveSeen(key: string | null, v: SeenState): void {
  if (!key) return;
  try {
    window.localStorage.setItem(SEEN_PREFIX + key, JSON.stringify(v));
  } catch {
    // As above.
  }
}

/** The list tile for a task: the board's state dot, in the board's words. */
/** The list's state for a task, the same lane and word the board shows. */
function taskTone(th: Thread | undefined, gate?: Gate | null): { tone: string; word: string } | null {
  if (!th) return null;
  const chrome = chromeFor(th, gate);
  return { tone: th.closed_without_review ? "unreviewed" : chrome.lane, word: th.closed_without_review ? "Unreviewed" : stateLabel(th, chrome) };
}

export function Threads() {
  const { session, store } = useSession();
  const deps = useMemo(() => {
    const creds = () => store.getSnapshot().credentials;
    const mode = () => store.getSnapshot().mode;
    const f = (url: string, init?: RequestInit) => fetch(url, init);
    return { threads: new ThreadsApi(f, creds, mode), board: new BoardApi(f, creds, mode) };
  }, [store]);
  const { data: board, selectChannel } = useBoard(session, deps.board);
  const { data: direct, refresh: refreshDirect } = useDirect(session, deps.threads);
  const open = useOpenRef();
  const [composing, setComposing] = useState(takeCompose);
  const [tab, setTab] = useState<Tab>(() => (composing || (open && open.kind !== "task") ? "direct" : "tasks"));
  // The palette's "New message" opens the composer.
  useEffect(() => {
    const open = () => {
      if (!takeCompose()) return;
      setTab("direct");
      setComposing(true);
    };
    window.addEventListener("maidan:compose", open);
    return () => window.removeEventListener("maidan:compose", open);
  }, []);
  // Opening a DM from a link shows it under Direct, a task under Tasks.
  const [seen, setSeen] = useState<ConvRef | null>(open);
  if (open !== seen && !sameRef(open, seen)) {
    setSeen(open);
    if (open) setTab(open.kind === "task" ? "tasks" : "direct");
  }
  const me = session.status === "signed-in" ? session.memberId : null;
  useVisualViewport(keyboardInset);
  const readsKey = session.status === "signed-in" ? `${session.workspaceId}:${session.memberId}` : null;
  const [reads, setReads] = useState<{ key: string | null; value: SeenState }>(() => ({ key: readsKey, value: loadSeen(readsKey) }));
  if (reads.key !== readsKey) setReads({ key: readsKey, value: loadSeen(readsKey) });
  const threadById = useMemo(() => new Map(board.threads.map((t) => [t.id, t])), [board.threads]);

  const tasks = useMemo(() => board.threads.map(taskConversation).sort(byRecent), [board.threads]);
  const dms = useMemo(() => {
    if (!me) return [];
    return [...direct.dms.map((d) => dmConversation(d, me, board.members)), ...direct.groups.map((g) => groupConversation(g, me, board.members))].sort(byRecent);
  }, [direct, me, board.members]);

  // A deep link can name a task in another channel or a DM not loaded yet.
  const current: Conversation | null = useMemo(() => {
    if (!open) return null;
    const known = [...tasks, ...dms].find((c) => sameRef(c.ref, open));
    if (known) return known;
    if (open.kind === "task") return { ref: open, threadId: open.id, label: "Task", detail: "", updatedAt: null };
    return null;
  }, [open, tasks, dms]);

  // Opening a conversation reads it, and so does staying in it as it moves.
  const openedId = current?.threadId ?? null;
  const openedAt = current?.updatedAt ?? null;
  const markKey = openedId ? `${openedId}@${openedAt ?? ""}` : null;
  const [marked, setMarked] = useState<string | null>(null);
  if (openedId && markKey !== marked) {
    setMarked(markKey);
    const now = new Date().toISOString();
    const value = markOpened(reads.value, openedId, openedAt && openedAt > now ? openedAt : now);
    saveSeen(reads.key, value);
    setReads({ ...reads, value });
  }

  if (session.status === "loading") return <p className="muted">Checking your sign-in…</p>;

  const list = tab === "tasks" ? tasks : dms;
  const listError = tab === "tasks" ? board.error : direct.error;
  const listLoading = tab === "tasks" ? board.status === "loading" || board.status === "idle" : direct.status === "loading" || direct.status === "idle";
  const waitingForDm = open && open.kind !== "task" && !current;

  return (
    <div className={open ? "threads has-open" : "threads"}>
      <section className="conv-list" aria-label="Conversations">
        <div className="conv-tools">
          <div className="seg" role="tablist" aria-label="Kind">
            <button role="tab" aria-selected={tab === "tasks"} className={tab === "tasks" ? "on" : ""} onClick={() => setTab("tasks")}>Tasks</button>
            <button role="tab" aria-selected={tab === "direct"} className={tab === "direct" ? "on" : ""} onClick={() => setTab("direct")}>Direct</button>
          </div>
          {tab === "tasks" ? (
            <label className="channel-pick">
              <span className="visually-hidden">Channel</span>
              <span className="channel-hash" aria-hidden="true">#</span>
              <select value={board.channelId ?? ""} onChange={(e) => selectChannel(e.target.value)} disabled={!board.channels.length}>
                {board.channels.map((c) => (
                  <option key={c.id} value={c.id}>{c.name}</option>
                ))}
              </select>
              <Icon name="chevron" />
            </label>
          ) : (
            <button className="btn small-btn" onClick={() => setComposing((v) => !v)} aria-expanded={composing}>{composing ? "Cancel" : "New message"}</button>
          )}
        </div>
        {tab === "direct" && composing && me && session.workspaceId && (
          <NewConversation
            api={deps.threads}
            me={me}
            workspaceId={session.workspaceId}
            members={board.members}
            onOpened={(ref) => {
              setComposing(false);
              void refreshDirect();
              window.location.hash = convHash(ref).slice(1);
            }}
          />
        )}
        {listError && <p className="conv-error" role="alert">{listError}</p>}
        {listLoading && !list.length ? (
          <div role="status" aria-label="Loading">
            <span className="skeleton row-sk" />
            <span className="skeleton row-sk" />
            <span className="skeleton row-sk" />
          </div>
        ) : !list.length ? (
          <p className="muted conv-empty">
            {tab === "tasks" ? (
              <>{emptyChannelText(board.channels.length)} <a href={NEW_WORK_HREF}>Start a task</a></>
            ) : "No conversations yet. Start one with New message."}
          </p>
        ) : (
          <ul className="convs">
            {list.map((c) => {
              const on = sameRef(c.ref, open);
              const unread = !on && isUnread(c, reads.value);
              const tone = c.ref.kind === "task" ? taskTone(threadById.get(c.ref.id), board.gates[c.ref.id]) : null;
              const th = c.ref.kind === "task" ? threadById.get(c.ref.id) : undefined;
              const who = th?.assignee_id ? board.members.get(th.assignee_id) : undefined;
              return (
                <li key={`${c.ref.kind}:${c.ref.id}`}>
                  <a className={`conv${on ? " on" : ""}${unread ? " unread" : ""}`} href={convHash(c.ref)} aria-current={on ? "page" : undefined}>
                    {c.ref.kind === "task" ? (
                      <span className={`conv-state state state-${tone?.tone ?? "open"}`} aria-hidden="true"><span className="state-dot" /></span>
                    ) : (
                      <span className={`avatar kind-${c.ref.kind}${c.ref.kind === "dm" && c.detail !== "Agent" ? " human" : ""}`} aria-hidden="true">{initials(c.label)}</span>
                    )}
                    <span className="conv-text">
                      <span className={c.label ? "conv-label" : "conv-label name-pending"}>{c.label}{unread && <span className="visually-hidden">, unread</span>}</span>
                      {tone ? (
                        <span className={`conv-detail state state-${tone.tone}`}>
                          <span className="state-text">{tone.word}</span>
                          {who && <span className="conv-who">· {who.name}</span>}
                        </span>
                      ) : (
                        <span className="conv-detail">{c.detail}</span>
                      )}
                    </span>
                    <span className="conv-end">
                      {c.updatedAt && <span className="conv-when">{clockTime(c.updatedAt)}</span>}
                      {unread && <span className="unread-dot" aria-hidden="true" />}
                    </span>
                  </a>
                </li>
              );
            })}
          </ul>
        )}
      </section>
      {open ? (
        current ? (
          <ConversationView key={`${current.ref.kind}:${current.ref.id}`} api={deps.threads} conv={current} me={me} members={board.members} />
        ) : (
          <section className="conv-open" aria-label="Conversation">
            <header className="conv-head">
              <a className="back" href="#/threads">Back</a>
            </header>
            <p className={direct.status === "ready" || direct.status === "error" ? "conv-error pad" : "muted pad"} role="status">
              {waitingForDm && (direct.status === "ready" || direct.status === "error") ? "This conversation is not one of yours, or it is gone." : "Loading…"}
            </p>
          </section>
        )
      ) : (
        <section className="conv-open conv-none" aria-hidden="true">
          <p className="muted">Pick a conversation.</p>
        </section>
      )}
    </div>
  );
}

function NewConversation({ api, me, workspaceId, members, onOpened }: { api: ThreadsApi; me: string; workspaceId: string; members: Map<string, Member>; onOpened: (ref: ConvRef) => void }) {
  const [picked, setPicked] = useState<string[]>([]);
  const [title, setTitle] = useState("");
  const [filter, setFilter] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const q = filter.trim().replace(/^@/, "").toLowerCase();
  const others = [...members.values()].filter((m) => m.id !== me && (m.name.toLowerCase().includes(q) || (m.handle ?? "").toLowerCase().includes(q)));
  const group = picked.length > 1;

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      if (group) {
        const refusal = groupRefusal(me, picked);
        if (refusal) throw new ThreadsError(refusal);
        const g = await api.openGroupDm(workspaceId, me, picked, title);
        onOpened({ kind: "group", id: g.id });
      } else {
        const dm = await api.openDm(workspaceId, me, picked[0] ?? null);
        onOpened({ kind: "dm", id: dm.id });
      }
    } catch (e) {
      setError(e instanceof ThreadsError ? e.message : "Could not reach the server");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className="new-conv"
      onSubmit={(e) => {
        e.preventDefault();
        void start();
      }}
    >
      <label className="field">
        <span>To</span>
        <input type="search" placeholder="Find a person or agent" aria-label="Find a person or agent by name or handle" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </label>
      <ul className="pick-list" aria-label="People">
        {others.map((m) => (
          <li key={m.id}>
            <label>
              <input
                type="checkbox"
                checked={picked.includes(m.id)}
                onChange={(e) => setPicked((p) => (e.target.checked ? [...p, m.id] : p.filter((x) => x !== m.id)))}
              />
              <span>{m.name}</span>
              {m.handle && m.handle !== m.name ? <span className="muted small">@{m.handle}</span> : null}
              <span className="tag">{m.kind === "agent" ? "Agent" : "Person"}</span>
            </label>
          </li>
        ))}
        {!others.length && <li className="muted small">No one matches.</li>}
      </ul>
      {group && (
        <label className="field">
          <span>Group name</span>
          <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Optional" />
        </label>
      )}
      {error && <p className="conv-error" role="alert">{error}</p>}
      <button type="submit" className="btn go" disabled={busy}>
        {busy ? "Opening…" : group ? `Start group of ${picked.length + 1}` : "Start conversation"}
      </button>
    </form>
  );
}

function ConversationView({ api, conv, me, members }: { api: ThreadsApi; conv: Conversation; me: string | null; members: Map<string, Member> }) {
  const { data, send, react, refresh } = useConversation(api, conv.threadId, me);
  const [pins, setPins] = useState<{ thread: string; ids: ReadonlySet<string> }>({ thread: conv.threadId, ids: new Set() });
  const pinned = pins.thread === conv.threadId ? pins.ids : EMPTY_PINS;
  const loadPins = useCallback(() => {
    const thread = conv.threadId;
    api.pins(thread).then(
      (ids) => setPins({ thread, ids: new Set(ids) }),
      () => undefined,
    );
  }, [api, conv.threadId]);
  useEffect(() => {
    const t = setTimeout(loadPins, 0);
    return () => clearTimeout(t);
  }, [loadPins]);
  const togglePin = async (id: string, on: boolean) => {
    try {
      await api.pin(conv.threadId, id, on);
      say(on ? "Pinned" : "Unpinned");
    } catch (e) {
      sayError(e, on ? "Could not pin that message" : "Could not unpin that message");
    } finally {
      loadPins();
    }
  };
  const removeMessage = async (m: Message, purge: boolean) => {
    try {
      await api.remove(m.id, purge);
      say(purge ? "Message purged" : "Message deleted");
      await refresh();
    } catch (e) {
      sayError(e, purge ? "Could not purge that message" : "Could not delete that message");
    }
  };
  const saveEdit = async (m: Message, body: string) => {
    await api.edit(m, body);
    say("Message edited");
    await refresh();
  };
  const [title, setTitle] = useState<string | null>(null);
  const messages = useMemo(() => visibleMessages(data.messages), [data.messages]);
  const scroller = useRef<HTMLDivElement>(null);
  const stuck = useRef(true);
  const sent = useRef(false);
  const prevIds = useRef<string[]>([]);
  const anchor = useRef<{ id: string; offset: number } | null>(null);
  const [unseen, setUnseen] = useState(0);

  // A deep-linked task in another channel names itself once it is read.
  useEffect(() => {
    if (conv.ref.kind !== "task" || conv.label !== "Task") return;
    let live = true;
    api.thread(conv.ref.id).then(
      (t) => live && setTitle(t.title || "(untitled task)"),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api, conv]);

  const remember = useCallback(() => {
    const el = scroller.current;
    if (!el) return;
    stuck.current = atBottom(el);
    if (stuck.current) setUnseen(0);
    const rows = el.querySelectorAll<HTMLElement>("[data-mid]");
    for (const r of rows) {
      if (r.offsetTop + r.offsetHeight > el.scrollTop) {
        anchor.current = { id: r.dataset.mid ?? "", offset: r.offsetTop - el.scrollTop };
        return;
      }
    }
  }, []);

  // New messages: follow the end only when the reader was there; otherwise
  // hold the row they were reading where it was and count what came below.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ids = messages.map((m) => m.id);
    const { follow, unseen: added } = onNewMessages(stuck.current, sent.current, prevIds.current, ids);
    prevIds.current = ids;
    sent.current = false;
    if (follow) {
      el.scrollTop = el.scrollHeight;
      stuck.current = true;
    } else if (anchor.current) {
      const row = el.querySelector<HTMLElement>(`[data-mid="${CSS.escape(anchor.current.id)}"]`);
      if (row) el.scrollTop = row.offsetTop - anchor.current.offset;
    }
    const t = setTimeout(() => setUnseen((n) => (follow ? 0 : n + added)), 0);
    return () => clearTimeout(t);
  }, [messages, data.reactions]);

  // The keyboard opening or the composer growing keeps the end in view.
  useEffect(() => {
    const el = scroller.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      if (stuck.current) el.scrollTop = el.scrollHeight;
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const toEnd = () => {
    const el = scroller.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    setUnseen(0);
  };

  const label = title ?? conv.label;

  return (
    <section className="conv-open" aria-label={label}>
      <header className="conv-head">
        <a className="back" href="#/threads">Back</a>
        <div className="conv-title">
          <h2>{label}</h2>
          {conv.detail && <p>{conv.detail}</p>}
        </div>
      </header>
      {conv.ref.kind === "task" && <TaskFactsPanel api={api} threadId={conv.threadId} members={members} />}
      {pinnedMessages(messages, pinned).length > 0 && (
        <details className="pins">
          <summary>Pinned ({pinnedMessages(messages, pinned).length})</summary>
          <ul>
            {pinnedMessages(messages, pinned).map((m) => (
              <li key={m.id}>
                <button
                  type="button"
                  className="link-btn"
                  onClick={() => scroller.current?.querySelector<HTMLElement>(`[data-mid="${CSS.escape(m.id)}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" })}
                >
                  <b className={members.size ? undefined : "name-pending"}>{nameOf(members, m.author_id)}</b>: {m.body.length > 90 ? `${m.body.slice(0, 90)}…` : m.body}
                </button>
              </li>
            ))}
          </ul>
        </details>
      )}
      {data.staleSince && <p className="conv-stale" role="status">Offline since {clockTime(new Date(data.staleSince).toISOString())}. Retrying.</p>}
      <div className="messages" ref={scroller} onScroll={remember} aria-live="polite" aria-relevant="additions">
        {data.status === "loading" || data.status === "idle" ? (
          <p className="muted pad">Loading messages…</p>
        ) : data.status === "error" ? (
          <p className="conv-error pad" role="alert">{data.error}</p>
        ) : !messages.length ? (
          <p className="muted pad">{conv.ref.kind === "task" ? "No messages yet. Start the conversation below." : "No messages"}</p>
        ) : (
          <ol className="msg-list">
            {messages.flatMap((m, i) => {
              const newDay = !messages[i - 1] || dayKey(messages[i - 1].posted_at) !== dayKey(m.posted_at);
              const row = (
              <MessageRow
                key={m.id}
                m={m}
                head={startsGroup(messages[i - 1], m)}
                members={members}
                mine={m.author_id === me}
                reactions={data.reactions[m.id] ?? []}
                onReact={react}
                pinned={pinned.has(m.id)}
                onPin={conv.ref.kind === "task" ? togglePin : null}
                onEdit={saveEdit}
                onRemove={removeMessage}
                api={api}
              />
              );
              return newDay
                ? [<li key={`day-${dayKey(m.posted_at)}`} className="day-sep"><span>{dayLabel(m.posted_at, Date.now())}</span></li>, row]
                : [row];
            })}
          </ol>
        )}
      </div>
      {unseen > 0 && (
        <button className="new-pill" onClick={toEnd}>
          {unseen === 1 ? "1 new message" : `${unseen} new messages`}
        </button>
      )}
      <Composer
        label={label}
        onAttach={(file) => api.attach(conv.threadId, file)}
        onSend={async (body) => {
          sent.current = true;
          await send(conv.ref, body);
        }}
      />
    </section>
  );
}

const EMPTY_PINS: ReadonlySet<string> = new Set();

type RowProps = {
  m: Message;
  head: boolean;
  members: Map<string, Member>;
  mine: boolean;
  reactions: Reaction[];
  onReact: (id: string, emoji: string, on: boolean) => Promise<void>;
  pinned: boolean;
  onPin: ((id: string, on: boolean) => Promise<void>) | null;
  onEdit: (m: Message, body: string) => Promise<void>;
  onRemove: (m: Message, purge: boolean) => Promise<void>;
  api: ThreadsApi;
};

function MessageRow({ m, head, members, mine, reactions, onReact, pinned, onPin, onEdit, onRemove, api }: RowProps) {
  // Destructive moves take two taps: the first arms, the second does it.
  const [armed, setArmed] = useState<"delete" | "purge" | null>(null);
  const [picking, setPicking] = useState(false);
  const [editing, setEditing] = useState(false);
  const [history, setHistory] = useState<Edit[] | "loading" | string | null>(null);
  const openHistory = () => {
    if (history) return setHistory(null);
    setHistory("loading");
    api.edits(m.id).then(
      (rows) => setHistory(rows),
      () => setHistory("Could not load the edit history."),
    );
  };
  const who = nameOf(members, m.author_id);
  const slash = slashResult(m.metadata);
  const toggle = (emoji: string, on: boolean) => {
    setPicking(false);
    void onReact(m.id, emoji, on).catch(() => undefined);
  };
  return (
    <li className={`msg${head ? " head" : ""}${mine ? " mine" : ""}${pinned ? " pinned" : ""}`} data-mid={m.id}>
      {head ? <span className={`avatar${members.get(m.author_id)?.kind === "human" ? " human" : ""}`} aria-hidden="true">{initials(who)}</span> : <span className="avatar-gap" aria-hidden="true" />}
      <div className="msg-main">
        {head && (
          <p className="msg-meta">
            <strong className={who ? undefined : "name-pending"}>{who}</strong> <time dateTime={m.posted_at}>{clockTime(m.posted_at)}</time>
            {m.edited_at && <span className="edited"> edited</span>}
            {pinned && <span className="edited"> pinned</span>}
          </p>
        )}
        {editing ? (
          <EditBox m={m} onCancel={() => setEditing(false)} onSave={async (body) => { await onEdit(m, body); setEditing(false); }} />
        ) : (
        <div
          className="msg-body"
          onClick={() => {
            // A phone has no hover, so a tap on the message offers reactions.
            if (window.matchMedia("(hover: none)").matches && !window.getSelection()?.toString()) setPicking((v) => !v);
          }}
        >
          <MessageBody body={m.body} />
        </div>
        )}
        {!head && (m.edited_at || pinned) && !editing && (
          <p className="msg-meta">{m.edited_at && <span className="edited">edited </span>}{pinned && <span className="edited">pinned</span>}</p>
        )}
        {history && (
          <div className="edit-history">
            {history === "loading" ? <p className="muted small">Loading history…</p> : typeof history === "string" ? <p className="err small">{history}</p> : !history.length ? <p className="muted small">No earlier versions.</p> : (
              <ol>
                {history.map((e, k) => (
                  <li key={k}>
                    <p className="muted small">{clockTime(e.edited_at)} · {nameOf(members, e.editor_id)}</p>
                    <p className="small"><del>{e.body_before}</del></p>
                    <p className="small">{e.body_after}</p>
                  </li>
                ))}
              </ol>
            )}
          </div>
        )}
        {slash && (
          <div className={`slash tone-${slash.tone}`}>
            <p className="slash-head">{slash.head}</p>
            <p className="slash-status">{slash.status}</p>
            {slash.response !== null && <pre>{slash.response}</pre>}
          </div>
        )}
        <div className={reactions.length || picking ? "reacts" : "reacts none"}>
          {reactions.map((r) => (
            <button key={r.emoji} className={r.mine ? "react on" : "react"} aria-pressed={r.mine} aria-label={`${REACTION_NAMES[r.emoji] ?? r.emoji}, ${r.count}${r.mine ? ", yours. Remove" : ". Add yours"}`} onClick={() => toggle(r.emoji, !r.mine)}>
              <span aria-hidden="true">{r.emoji}</span> {r.count}
            </button>
          ))}
          <button className="react add" aria-expanded={picking} aria-label="Add a reaction" onClick={() => setPicking((v) => !v)}>
            <span aria-hidden="true">+</span>
          </button>
          {picking &&
            QUICK_REACTIONS.filter((e) => !reactions.some((r) => r.emoji === e && r.mine)).map((e) => (
              <button key={e} className="react" aria-label={`React with ${REACTION_NAMES[e]}`} onClick={() => toggle(e, true)}>
                <span aria-hidden="true">{e}</span>
              </button>
            ))}
          {picking && onPin && (
            <button className="react word" onClick={() => { setPicking(false); void onPin(m.id, !pinned); }}>{pinned ? "Unpin" : "Pin"}</button>
          )}
          {picking && mine && !m.tombstoned_at && (
            <button className="react word" onClick={() => { setPicking(false); setEditing(true); }}>Edit</button>
          )}
          {picking && m.edited_at && (
            <button className="react word" aria-expanded={Boolean(history)} onClick={() => { setPicking(false); openHistory(); }}>History</button>
          )}
          {picking && mine && !m.tombstoned_at && (
            armed === "delete"
              ? <button className="react word danger" onClick={() => { setPicking(false); setArmed(null); void onRemove(m, false); }}>Delete? Tap again</button>
              : <button className="react word" onClick={() => setArmed("delete")}>Delete</button>
          )}
          {picking && m.tombstoned_at && (
            armed === "purge"
              ? <button className="react word danger" onClick={() => { setPicking(false); setArmed(null); void onRemove(m, true); }}>Purge for good? Tap again</button>
              : <button className="react word" onClick={() => setArmed("purge")}>Purge</button>
          )}
        </div>
      </div>
    </li>
  );
}

function EditBox({ m, onSave, onCancel }: { m: Message; onSave: (body: string) => Promise<void>; onCancel: () => void }) {
  const [draft, setDraft] = useState(m.body);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const why = editRefusal(m.body, draft);
  return (
    <form
      className="edit-box"
      onSubmit={(e) => {
        e.preventDefault();
        if (busy || why) return;
        setBusy(true);
        setError(null);
        onSave(draft).then(
          () => setBusy(false),
          (err: unknown) => {
            setBusy(false);
            setError(err instanceof Error && err.message ? err.message : "Could not save the edit. Your text is still here.");
          },
        );
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") onCancel();
      }}
    >
      <label className="visually-hidden" htmlFor={`edit-${m.id}`}>Edit message</label>
      <textarea id={`edit-${m.id}`} value={draft} rows={2} autoFocus onChange={(e) => setDraft(e.target.value)} />
      {error && <p className="conv-error" role="alert">{error}</p>}
      <div className="edit-actions">
        <button type="button" className="chip-btn" onClick={onCancel}>Cancel</button>
        <button type="submit" className="chip-btn confirm" disabled={busy || Boolean(why)}>{busy ? "Saving" : "Save"}</button>
      </div>
    </form>
  );
}

type ComposerProps = {
  label: string;
  onSend: (body: string) => Promise<void>;
  onAttach: (file: File) => Promise<{ filename: string }>;
};

function Composer({ label, onSend, onAttach }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attached, setAttached] = useState<string | null>(null);
  const box = useRef<HTMLTextAreaElement>(null);

  // A pasted file (a screenshot, usually) is stored and linked to this
  // conversation's thread; pasted text is left alone.
  const paste = async (files: File[]) => {
    setError(null);
    setAttached(null);
    const names: string[] = [];
    for (const f of files) {
      try {
        names.push((await onAttach(f)).filename);
      } catch (e) {
        setError(e instanceof ThreadsError ? e.message : "Could not attach that file.");
        break;
      }
    }
    if (names.length) setAttached(`Attached ${names.join(", ")}`);
  };

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSend(draft);
      setDraft("");
    } catch (e) {
      // The draft stays, so nothing typed is lost.
      setError(e instanceof ThreadsError ? e.message : "Could not reach the server. Your message is still here.");
    } finally {
      setBusy(false);
      box.current?.focus();
    }
  };

  return (
    <form
      className="composer"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      {error && <p className="conv-error" role="alert">{error}</p>}
      <p className="composer-note" role="status">{attached ?? ""}</p>
      <div className="composer-row">
        <label className="visually-hidden" htmlFor="compose">Message {label}</label>
        <textarea
          id="compose"
          ref={box}
          rows={1}
          value={draft}
          placeholder="Message"
          enterKeyHint="send"
          onChange={(e) => setDraft(e.target.value)}
          onPaste={(e) => {
            const files = pastedFiles(e.clipboardData);
            if (!files.length) return;
            e.preventDefault();
            void paste(files);
          }}
          onKeyDown={(e) => {
            // A keyboard with a pointer sends on Enter; a phone keeps Enter for a new line.
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && window.matchMedia("(pointer: fine)").matches) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <button type="submit" className="btn go send" disabled={busy || !draft.trim()}>
          {busy ? "Sending" : "Send"}
        </button>
      </div>
    </form>
  );
}

function TaskFactsPanel({ api, threadId, members }: { api: ThreadsApi; threadId: string; members: Map<string, Member> }) {
  const [facts, setFacts] = useState<{ thread: string; value: TaskFacts } | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open || facts?.thread === threadId) return;
    let live = true;
    void api.facts(threadId).then((value) => {
      if (live) setFacts({ thread: threadId, value });
    });
    return () => {
      live = false;
    };
  }, [api, threadId, open, facts?.thread]);
  const f = facts?.thread === threadId ? facts.value : null;
  const name = (id: string | undefined) => (id ? members.get(id)?.name || (members.size ? id : "…") : "someone");
  const deps = f?.deps?.dependencies ?? [];
  const entries = withRecords(f?.evidence?.entries ?? [], f?.records ?? null);
  const reload = () => setFacts(null);
  return (
    <details className="pins facts-panel" onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}>
      <summary>Result and evidence</summary>
      {!f ? (
        <p className="muted small">Loading…</p>
      ) : (
        <div className="facts-body small">
          <dl className="fact-rows">
            <div><dt>Result</dt><dd>{f.result ? <><pre className="fact-code fact-json" tabIndex={0}>{JSON.stringify(f.result.result, null, 2)}</pre> by {name(f.result.produced_by ?? undefined)}</> : <span className="muted">None recorded yet</span>}</dd></div>
            <div><dt>Depends on</dt><dd>{deps.length ? deps.map((d) => d.depends_on_thread_id).join(", ") : <span className="muted">Nothing</span>}{f.deps ? ` · ${f.deps.ready ? "ready" : "not ready"}` : ""}</dd></div>
          </dl>
          {f.evidence?.evidence_changed_since_review ? <p className="fact-changed">Evidence changed since the last review.</p> : null}
          {entries.length ? (
            <ol className="ev-timeline" aria-label="Evidence">
              {entries.map((e, i) => {
                const href = e.kind === "evidence_recorded" ? safeHref(e.record?.url) : null;
                return (
                  <li key={e.record?.id ?? i}>
                    <span>
                      {evidenceLine(e, name)}
                      {href ? <> · <a href={href} target="_blank" rel="noopener noreferrer">open</a></> : null}
                    </span>
                    <time dateTime={e.at}>{clockTime(e.at)}</time>
                  </li>
                );
              })}
            </ol>
          ) : <p className="muted">No evidence recorded yet.</p>}
          <AddEvidence api={api} threadId={threadId} onAdded={reload} />
        </div>
      )}
    </details>
  );
}

const EMPTY_DRAFT: EvidenceDraft = { kind: "test_run", source: "", url: "", sha256: "", note: "" };

/** Attach a test run, CI run, diff, outside review, screenshot or note to the task. */
function AddEvidence({ api, threadId, onAdded }: { api: ThreadsApi; threadId: string; onAdded: () => void }) {
  const [draft, setDraft] = useState<EvidenceDraft>(EMPTY_DRAFT);
  const [busy, setBusy] = useState(false);
  const why = evidenceRefusal(draft);
  const set = (k: keyof EvidenceDraft) => (e: { currentTarget: { value: string } }) => {
    const value = e.currentTarget.value;
    setDraft((d) => ({ ...d, [k]: value }));
  };
  const submit = async () => {
    if (why || busy) return;
    setBusy(true);
    try {
      await api.addEvidence(threadId, draft);
      say("Evidence recorded");
      setDraft(EMPTY_DRAFT);
      onAdded();
    } catch (err) {
      sayError(err, "Could not record the evidence");
    } finally {
      setBusy(false);
    }
  };
  return (
    <details className="add-evidence">
      <summary>Add evidence</summary>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <label>
          Kind{" "}
          <select value={draft.kind} onChange={(e) => setDraft((d) => ({ ...d, kind: e.currentTarget.value as EvidenceKind }))}>
            {EVIDENCE_KINDS.map((k) => <option key={k} value={k}>{EVIDENCE_KIND_WORD[k]}</option>)}
          </select>
        </label>
        <label>Source <input value={draft.source} onChange={set("source")} placeholder="cargo test, github-actions" maxLength={200} /></label>
        <label>URL <input type="url" value={draft.url} onChange={set("url")} placeholder="https://" /></label>
        <label>sha256 <input value={draft.sha256} onChange={set("sha256")} spellCheck={false} /></label>
        <label>Note <textarea value={draft.note} onChange={set("note")} rows={2} maxLength={4000} /></label>
        {why && (draft.source || draft.url || draft.sha256 || draft.note) ? <p className="muted">{why}</p> : null}
        <button type="submit" className="btn" disabled={busy || Boolean(why)}>{busy ? "Recording" : "Record"}</button>
        <p className="muted">Records are kept as written; they cannot be edited or removed. Adding one needs a bearer token.</p>
      </form>
    </details>
  );
}
