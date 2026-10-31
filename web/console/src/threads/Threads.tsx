// Threads and DMs, iPhone first: a list of conversations, and one open
// conversation that fills the screen with its composer above the keyboard.
// On a wide screen the list and the conversation sit side by side.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { BoardApi, type Member } from "../core/board.ts";
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
  sameRef,
  slashResult,
  startsGroup,
  taskConversation,
  visibleMessages,
  type ConvRef,
  type Conversation,
  type Edit,
  type TaskFacts,
  evidenceLine,
  type Message,
  type Reaction,
} from "../core/threads.ts";
import { say, sayError } from "../core/notices.ts";
import { useBoard } from "../hooks/useBoard.ts";
import { useSession } from "../hooks/useSession.ts";
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
  const [tab, setTab] = useState<Tab>(() => (open && open.kind !== "task" ? "direct" : "tasks"));
  const [composing, setComposing] = useState(false);
  // Opening a DM from a link shows it under Direct, a task under Tasks.
  const [seen, setSeen] = useState<ConvRef | null>(open);
  if (open !== seen && !sameRef(open, seen)) {
    setSeen(open);
    if (open) setTab(open.kind === "task" ? "tasks" : "direct");
  }
  const me = session.status === "signed-in" ? session.memberId : null;
  useVisualViewport(keyboardInset);

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

  if (session.status === "loading") return <p className="muted">Checking your sign-in…</p>;
  if (session.status === "signed-out")
    return (
      <div className="card">
        <h2>Sign in to read your threads</h2>
        <p className="muted">Sign in on the classic board, then come back here.</p>
        <a className="button steel" href="/ui/">Open the classic board</a>
      </div>
    );

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
              <select value={board.channelId ?? ""} onChange={(e) => selectChannel(e.target.value)} disabled={!board.channels.length}>
                {board.channels.map((c) => (
                  <option key={c.id} value={c.id}>#{c.name}</option>
                ))}
              </select>
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
          <p className="muted">Loading…</p>
        ) : !list.length ? (
          <p className="muted conv-empty">{tab === "tasks" ? "No tasks in this channel yet." : "No conversations yet. Start one with New message."}</p>
        ) : (
          <ul className="convs">
            {list.map((c) => (
              <li key={`${c.ref.kind}:${c.ref.id}`}>
                <a className={sameRef(c.ref, open) ? "conv on" : "conv"} href={convHash(c.ref)} aria-current={sameRef(c.ref, open) ? "page" : undefined}>
                  <span className={`avatar kind-${c.ref.kind}`} aria-hidden="true">{c.ref.kind === "task" ? "#" : initials(c.label)}</span>
                  <span className="conv-text">
                    <span className="conv-label">{c.label}</span>
                    <span className="conv-detail">{c.detail}</span>
                  </span>
                  {c.updatedAt && <span className="conv-when">{clockTime(c.updatedAt)}</span>}
                </a>
              </li>
            ))}
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
  const others = [...members.values()].filter((m) => m.id !== me && m.name.toLowerCase().includes(filter.trim().toLowerCase()));
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
        <input type="search" placeholder="Find a person or agent" value={filter} onChange={(e) => setFilter(e.target.value)} />
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
              {m.kind === "agent" && <span className="tag">Agent</span>}
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
                  <b>{nameOf(members, m.author_id)}</b>: {m.body.length > 90 ? `${m.body.slice(0, 90)}…` : m.body}
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
            {messages.map((m, i) => (
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
                api={api}
              />
            ))}
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
  api: ThreadsApi;
};

function MessageRow({ m, head, members, mine, reactions, onReact, pinned, onPin, onEdit, api }: RowProps) {
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
      {head ? <span className="avatar" aria-hidden="true">{initials(who)}</span> : <span className="avatar-gap" aria-hidden="true" />}
      <div className="msg-main">
        {head && (
          <p className="msg-meta">
            <strong>{who}</strong> <time dateTime={m.posted_at}>{clockTime(m.posted_at)}</time>
            {m.edited_at && <span className="edited"> edited</span>}
            {pinned && <span className="edited"> pinned</span>}
          </p>
        )}
        {editing ? (
          <EditBox m={m} onCancel={() => setEditing(false)} onSave={async (body) => { await onEdit(m, body); setEditing(false); }} />
        ) : (
        <p
          className="msg-body"
          onClick={() => {
            // A phone has no hover, so a tap on the message offers reactions.
            if (window.matchMedia("(hover: none)").matches && !window.getSelection()?.toString()) setPicking((v) => !v);
          }}
        >
          {m.body}
        </p>
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

function Composer({ label, onSend }: { label: string; onSend: (body: string) => Promise<void> }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLTextAreaElement>(null);

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
  const name = (id: string | undefined) => (id ? members.get(id)?.name || id : "someone");
  const deps = f?.deps?.dependencies ?? [];
  const entries = f?.evidence?.entries ?? [];
  return (
    <details className="pins facts-panel" onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}>
      <summary>Result and evidence</summary>
      {!f ? (
        <p className="muted small">Loading…</p>
      ) : (
        <div className="small">
          <p><b>Result</b>: {f.result ? <><code>{JSON.stringify(f.result.result)}</code> by {name(f.result.produced_by ?? undefined)}</> : "none recorded yet"}</p>
          <p><b>Depends on</b>: {deps.length ? deps.map((d) => d.depends_on_thread_id).join(", ") : "nothing"}{f.deps ? ` · ${f.deps.ready ? "ready" : "not ready"}` : ""}</p>
          {f.evidence?.evidence_changed_since_review ? <p className="err">Evidence changed since the last review.</p> : null}
          {entries.length ? (
            <ol className="edit-history">
              {entries.map((e, i) => <li key={i}><time dateTime={e.at}>{new Date(e.at).toLocaleString()}</time> · {evidenceLine(e, name)}</li>)}
            </ol>
          ) : <p className="muted">No evidence recorded yet.</p>}
        </div>
      )}
    </details>
  );
}
