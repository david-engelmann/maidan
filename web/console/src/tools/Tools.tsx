// Tools: what the classic board kept on its Tools tabs, minus the gates
// Needs you already answers. Inbox and Preferences are the signed-in
// member's own; Deliveries and Commands are the workspace's; Explore reads
// queues, schedules, events and peers; Operator is the server-wide audit and
// reindex, listed only for a credential that holds them;
// Search finds messages; Workspace renames, switches, and adds channels,
// tasks, agents and peers, and reads apps and the workspace audit.
// Recent is every notice this tab has shown.
import { type FormEvent, type KeyboardEvent, type ReactNode, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { ago, BoardApi, type Thread } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import { PeopleApi, type Person } from "../core/people.ts";
import { convHash, ThreadsApi } from "../core/threads.ts";
import { notices, say, sayError } from "../core/notices.ts";
import {
  DELIVERY_KINDS,
  EVENT_KINDS,
  HANDLER_KINDS,
  HANDLE_REQUIRED,
  SEARCH_MODES,
  handleRefusal,
  peerRefusal,
  PEERS_NEED,
  AUDIT_READ_GLOBAL,
  FEDERATION_ADMIN,
  OPERATOR_GLOBAL,
  holds,
  showsOperator,
  channelFollowLabel,
  channelsToFollow,
  describeNotificationKind,
  memberChoices,
  memberLabel,
  searchQuery,
  switchHref,
  taskFollowLabel,
  tasksToFollow,
  threadLinkLabel,
  catalogIsFresh,
  catalogShouldRefresh,
  nextCatalog,
  type TaskCatalogRead,
  type TaskCatalogView,
  NOTIFICATION_KINDS,
  deliveryState,
  scheduleCadence,
  type DeliveryStatus,
  type PickerChannel,
  type PickerTask,
  type ReindexJob,
  type ToolsApi,
  type WorkEvent,
} from "../core/tools.ts";
import type { Loaded } from "../hooks/usePeople.ts";
import { useSession } from "../hooks/useSession.ts";
import { MODE_LINE } from "../core/api.ts";
import { capabilityCard } from "../core/session.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";
import { useTheme } from "../hooks/useTheme.ts";
import { THEME_CHOICES } from "../core/theme.ts";
import { consolePushManager, installStore, promptInstall } from "../pwa/install.ts";
import { PUSH_WORDS, disablePush, enablePush, pushRefusal, pushSubscribed, type PushOutcome } from "../pwa/push.ts";
import { peekPaletteIndex } from "../shell/paletteIndex.ts";
import { Connect } from "./Connect.tsx";
import { SecretOnce } from "./SecretOnce.tsx";

export const TOOL_TABS = [
  { id: "inbox", title: "Inbox" },
  { id: "prefs", title: "Preferences" },
  { id: "deliveries", title: "Deliveries" },
  { id: "commands", title: "Commands" },
  { id: "search", title: "Search" },
  { id: "workspace", title: "Workspace" },
  { id: "explore", title: "Explore" },
  { id: "operator", title: "Operator" },
  { id: "recent", title: "Recent" },
  { id: "device", title: "This device" },
  { id: "connect", title: "Connect" },
] as const;
type ToolTab = (typeof TOOL_TABS)[number]["id"];

function subscribeHash(fn: () => void): () => void {
  window.addEventListener("hashchange", fn);
  return () => window.removeEventListener("hashchange", fn);
}

export function toolTab(hash: string): ToolTab {
  const id = hash.replace(/^#\/?/, "").split("/")[1];
  return TOOL_TABS.find((t) => t.id === id)?.id ?? "inbox";
}

export function Tools() {
  const { session } = useSession();
  const api = useToolsApi();
  const tab = toolTab(useSyncExternalStore(subscribeHash, () => window.location.hash));
  const tabs = useRef<HTMLDivElement>(null);
  // Keep the open tab in view when the strip scrolls sideways on a phone.
  useEffect(() => {
    tabs.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "center" });
  }, [tab]);
  const me = session.status === "signed-in" ? session.memberId : null;
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  const operator = showsOperator(session.grant);
  return (
    <div className="tools">
      <div className="seg-wrap">
      <div ref={tabs} className="seg seg-scroll" role="tablist" aria-label="Tools">
        {TOOL_TABS.filter((t) => t.id !== "operator" || operator).map((t) => (
          <a key={t.id} role="tab" aria-selected={tab === t.id} href={`#/tools/${t.id}`}>{t.title}</a>
        ))}
      </div>
      </div>
      {tab === "inbox" ? <Inbox api={api} me={me} ws={ws} /> : null}
      {tab === "prefs" ? <Prefs api={api} me={me} ws={ws} /> : null}
      {tab === "deliveries" ? <Deliveries api={api} ws={ws} /> : null}
      {tab === "commands" ? <Commands api={api} ws={ws} /> : null}
      {tab === "search" ? <Search api={api} ws={ws} /> : null}
      {tab === "workspace" ? <Workspace api={api} ws={ws} /> : null}
      {tab === "explore" ? <Explore api={api} ws={ws} /> : null}
      {tab === "operator" ? operator ? <Operator api={api} ws={ws} /> : <Panel title="Operator"><p className="muted">These tools are for whoever runs this server. Your sign-in does not include them.</p></Panel> : null}
      {tab === "recent" ? <Recent /> : null}
      {tab === "device" ? <Device api={api} me={me} /> : null}
      {tab === "connect" ? <Connect api={api} base={session.credentials.base || window.location.origin} signedIn={session.status === "signed-in"} /> : null}
    </div>
  );
}

function Panel({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="card tool-panel">
      <div className="panel-head">
        <h2>{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

function State<T>({ loaded, empty, children }: { loaded: Loaded<T> & { reload?: () => Promise<void> }; empty?: (v: T) => boolean; children: (v: T) => ReactNode }) {
  if (loaded.status === "loading") return <p className="muted">Loading…</p>;
  if (loaded.value === null) {
    const reload = loaded.reload;
    return (
      <div className="load-failed" role="alert">
        <p className="err">{loaded.error}</p>
        {reload ? <button type="button" className="btn" onClick={() => void reload()}>Try again</button> : null}
      </div>
    );
  }
  return (
    <>
      {loaded.error ? <p className="err small" role="alert">{loaded.error}</p> : null}
      {empty && empty(loaded.value) ? null : children(loaded.value)}
    </>
  );
}

/** Runs a write once at a time and says how it went. */
function useAct() {
  const [busy, setBusy] = useState<string | null>(null);
  const act = async (id: string, run: () => Promise<unknown>, done: string | null, fallback: string) => {
    if (busy) return;
    setBusy(id);
    try {
      await run();
      if (done) say(done);
    } catch (e) {
      sayError(e, fallback);
    } finally {
      setBusy(null);
    }
  };
  return { busy, act };
}

const SELF_ONLY = <p className="muted">Sign in as a member to see this.</p>;

/** A few channels at a time, so opening Tools does not request every thread list at once. */
const TASK_FETCHES = 4;
const EMPTY_TASKS: ReadonlyMap<string, PickerTask> = new Map();

type TaskCatalog = TaskCatalogView;
type StampedCatalog = { view: TaskCatalog; at: number };
type WorkspaceTasks = { tasks: PickerTask[]; byId: ReadonlyMap<string, PickerTask>; status: "loading" | "ready" | "error" };

const taskCatalogs = new Map<string, StampedCatalog>();
const taskCatalogJobs = new Map<string, Promise<TaskCatalog>>();
const taskCatalogEpoch = new Map<string, number>();
/** Missing threads, including a 404, so a render does not ask for the same id again. */
const resolvedThreads = new Map<string, Map<string, PickerTask | null>>();
const resolvingThreads = new Map<string, Promise<void>>();

function catalogFromPalette(ws: string): TaskCatalog | null {
  const index = peekPaletteIndex(ws);
  if (!index) return null;
  const tasks: PickerTask[] = [];
  for (const task of index.tasks) {
    const title = task.title.trim();
    if (!title) continue;
    tasks.push({ id: task.id, title, channelName: task.channelName?.trim() || null });
  }
  if (!tasks.length) return null;
  return { tasks, channels: index.channels.map((channel) => ({ id: channel.id, name: channel.name })), error: false };
}

function peekCatalog(ws: string, now = Date.now()): TaskCatalog | null {
  const cached = taskCatalogs.get(ws);
  // A fresh complete read is shown at once. A stale one is still shown while
  // the mount asks again, and a palette snapshot is never written into the map.
  if (cached) return catalogIsFresh(cached.at, now) || cached.view.tasks.length > 0 ? cached.view : null;
  return catalogFromPalette(ws);
}

const NO_EXTRAS: PickerTask[] = [];

function extraTasks(ws: string): PickerTask[] {
  const found = resolvedThreads.get(ws);
  if (!found) return NO_EXTRAS;
  const tasks: PickerTask[] = [];
  for (const task of found.values()) if (task?.title.trim()) tasks.push(task);
  return tasks.length ? tasks : NO_EXTRAS;
}

async function pool<T>(items: readonly T[], limit: number, run: (item: T) => Promise<void>): Promise<void> {
  if (items.length === 0) return;
  let next = 0;
  const workers = Math.min(limit, items.length);
  await Promise.all(Array.from({ length: workers }, async () => {
    for (;;) {
      const index = next;
      next += 1;
      if (index >= items.length) return;
      await run(items[index]);
    }
  }));
}

async function fetchCatalog(ws: string, tools: ToolsApi, board: BoardApi): Promise<TaskCatalogRead> {
  try {
    const channels = await tools.channels(ws);
    const tasks: PickerTask[] = [];
    let failed = 0;
    await pool(channels, TASK_FETCHES, async (channel) => {
      try {
        const rows = await board.threads(channel.id);
        for (const row of rows) {
          const title = (row.title ?? "").trim();
          if (!title) continue;
          tasks.push({ id: row.id, title, channelId: channel.id, channelName: channel.name.trim() || null });
        }
      } catch {
        failed += 1;
      }
    });
    const total = channels.length > 0 && failed === channels.length;
    return { tasks, channels, failed, error: total ? "The task list did not load" : null };
  } catch {
    return { tasks: [], channels: [], failed: 0, error: "The task list did not load" };
  }
}

function loadCatalog(ws: string, tools: ToolsApi, board: BoardApi): Promise<TaskCatalog> {
  const pending = taskCatalogJobs.get(ws);
  if (pending) return pending;
  const epoch = taskCatalogEpoch.get(ws) ?? 0;
  const job = fetchCatalog(ws, tools, board).then((read) => {
    const stamped = taskCatalogs.get(ws);
    if ((taskCatalogEpoch.get(ws) ?? 0) !== epoch) return stamped?.view ?? nextCatalog(null, read).show;
    const next = nextCatalog(stamped?.view ?? null, read);
    if (next.store) taskCatalogs.set(ws, { view: next.store, at: Date.now() });
    else if (read.failed > 0 || read.error !== null) taskCatalogs.delete(ws);
    return next.show;
  }).finally(() => {
    if (taskCatalogJobs.get(ws) === job) taskCatalogJobs.delete(ws);
  });
  taskCatalogJobs.set(ws, job);
  return job;
}

function channelOf(thread: Thread): string | null {
  const value = (thread as { channel_id?: unknown }).channel_id;
  return typeof value === "string" && value ? value : null;
}

function pickerFromThread(thread: Thread, channels: ReadonlyMap<string, string>): PickerTask | null {
  const title = (thread.title ?? "").trim();
  if (!title) return null;
  const channelId = channelOf(thread);
  const channelName = channelId ? channels.get(channelId)?.trim() || null : null;
  return { id: thread.id, title, channelId, channelName };
}

function rememberThread(ws: string, id: string, task: PickerTask | null): void {
  let found = resolvedThreads.get(ws);
  if (!found) {
    found = new Map();
    resolvedThreads.set(ws, found);
  }
  found.set(id, task);
}

/** One read per thread id. A direct message or a subthread is not in the channel lists. */
function resolveMissing(ws: string, id: string, api: ThreadsApi, channels: ReadonlyMap<string, string>): Promise<void> {
  if (resolvedThreads.get(ws)?.has(id)) return Promise.resolve();
  const key = `${ws}\0${id}`;
  const pending = resolvingThreads.get(key);
  if (pending) return pending;
  const job = api.thread(id).then(
    (thread) => rememberThread(ws, id, pickerFromThread(thread, channels)),
    () => rememberThread(ws, id, null),
  ).finally(() => {
    if (resolvingThreads.get(key) === job) resolvingThreads.delete(key);
  });
  resolvingThreads.set(key, job);
  return job;
}

function usePeopleApi(): PeopleApi {
  const { store } = useSession();
  return useMemo(() => new PeopleApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
}

/**
 * Channel tasks for this workspace, from the palette when it already has them,
 * otherwise from each channel a few at a time. Thread ids the lists do not
 * carry are resolved once and remembered, including a miss.
 */
function useWorkspaceTasks(ws: string | null, api: ToolsApi, referenced: readonly string[]): WorkspaceTasks {
  const { store } = useSession();
  const board = useMemo(() => new BoardApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const threads = useMemo(() => new ThreadsApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const refKey = [...new Set(referenced.map((id) => id.trim()).filter(Boolean))].sort().join("\0");
  const [fetched, setFetched] = useState<{ ws: string; value: TaskCatalog } | null>(null);
  const [extras, setExtras] = useState<{ ws: string; tasks: PickerTask[] }>({ ws: "", tasks: NO_EXTRAS });
  const [rev, setRev] = useState(0);
  const warm = useMemo(() => (ws ? peekCatalog(ws) : null), [ws, rev]);
  const held = (fetched?.ws === ws ? fetched.value : null) ?? warm;
  const shownExtras = useMemo(() => (extras.ws === ws ? extras.tasks : ws ? extraTasks(ws) : NO_EXTRAS), [extras, ws]);
  useEffect(() => {
    if (!ws) return;
    return liveBus.listen((m) => {
      if (!catalogShouldRefresh(m)) return;
      taskCatalogs.delete(ws);
      taskCatalogJobs.delete(ws);
      taskCatalogEpoch.set(ws, (taskCatalogEpoch.get(ws) ?? 0) + 1);
      resolvedThreads.delete(ws);
      setRev((n) => n + 1);
    });
  }, [ws]);
  useEffect(() => {
    if (!ws) return;
    let alive = true;
    void loadCatalog(ws, api, board).then((value) => {
      if (alive) setFetched({ ws, value });
    });
    return () => {
      alive = false;
    };
  }, [ws, api, board, rev]);
  useEffect(() => {
    if (!ws || !held) return;
    const ids = refKey ? refKey.split("\0") : [];
    const known = new Set(held.error ? [] : held.tasks.map((task) => task.id));
    const names = new Map(held.channels.map((channel) => [channel.id, channel.name]));
    const todo = ids.filter((id) => !known.has(id) && !resolvedThreads.get(ws)?.has(id));
    if (!todo.length) return;
    let alive = true;
    const publish = () => {
      if (alive) setExtras({ ws, tasks: extraTasks(ws) });
    };
    void pool(todo, TASK_FETCHES, (id) => resolveMissing(ws, id, threads, names)).then(publish, publish);
    return () => {
      alive = false;
    };
  }, [ws, held, refKey, threads]);
  const byId = useMemo(() => {
    const map = new Map<string, PickerTask>();
    for (const task of shownExtras) map.set(task.id, task);
    if (held && !held.error) for (const task of held.tasks) if (task.title.trim()) map.set(task.id, task);
    return map;
  }, [held, shownExtras]);
  if (!ws) return { tasks: [], byId: EMPTY_TASKS, status: "ready" };
  if (!held) return { tasks: [], byId: EMPTY_TASKS, status: "loading" };
  if (held.error) return { tasks: [], byId, status: "error" };
  return { tasks: held.tasks, byId, status: "ready" };
}

function namedMember(people: readonly { id: string; name: string; handle?: string | null }[] | null | undefined, id: string | null | undefined): string | null {
  if (!id || !people) return null;
  const member = people.find((person) => person.id === id);
  return member ? memberLabel(member) : null;
}

function memberBy(people: readonly { id: string; name: string; handle?: string | null }[] | null | undefined, id: string | null | undefined): string {
  const name = namedMember(people, id);
  return name ? ` \u00b7 by ${name}` : "";
}

function auditActor(id: string, people: Loaded<Person[]>): string | null {
  const named = namedMember(people.value, id);
  if (named) return named;
  if (people.status === "loading") return null;
  return id.slice(0, 8);
}

function eventTarget(event: WorkEvent, tasks: ReadonlyMap<string, PickerTask>, channels: PickerChannel[]): ReactNode {
  if (event.thread_id) {
    const fallback = `thread ${event.thread_id.slice(0, 8)}`;
    return <a href={convHash({ kind: "task", id: event.thread_id })}>{threadLinkLabel(event.thread_id, tasks, fallback)}</a>;
  }
  if (event.channel_id) {
    const named = channelFollowLabel(event.channel_id, channels);
    return named === event.channel_id ? `channel ${event.channel_id.slice(0, 8)}` : named;
  }
  return "workspace";
}

// ---- Inbox ---------------------------------------------------------------

function Inbox({ api, me, ws }: { api: ToolsApi; me: string | null; ws: string | null }) {
  const [unreadOnly, setUnreadOnly] = useState(true);
  const list = useLoad(me ? `${me}|${unreadOnly}` : null, () => api.notifications(me ?? "", unreadOnly));
  const count = useLoad(me ? `c|${me}|${list.value?.length ?? -1}` : null, () => api.unread(me ?? ""));
  const { busy, act } = useAct();
  const threadIds = (list.value ?? []).flatMap((n) => (n.thread_id ? [n.thread_id] : []));
  const tasks = useWorkspaceTasks(ws, api, threadIds);
  if (!me) return <Panel title="Inbox">{SELF_ONLY}</Panel>;
  const reload = async () => {
    await list.reload();
    await count.reload();
  };
  return (
    <Panel
      title={count.value != null ? `Inbox · ${count.value} unread` : "Inbox"}
      action={<button type="button" className="chip-btn" disabled={busy !== null || !count.value} onClick={() => void act("all", async () => { await api.markAllRead(me); await reload(); }, "Every notification is marked read", "Could not mark them read")}>Mark all read</button>}
    >
      <label className="check"><input type="checkbox" checked={unreadOnly} onChange={(e) => setUnreadOnly(e.target.checked)} /> Unread only</label>
      <State loaded={list}>
        {(rows) =>
          rows.length ? (
            <ul className="tool-list">
              {rows.map((n) => (
                <li key={n.id} tabIndex={-1} className={`tool-row${n.read_at ? " read" : ""}`}>
                  <div>
                    <b>{describeNotificationKind(n.kind)}</b>
                    <p className="muted small">{n.thread_id ? <a href={convHash({ kind: "task", id: n.thread_id })}>{threadLinkLabel(n.thread_id, tasks.byId, "Open the thread")}</a> : "Workspace"}{n.created_at ? ` · ${ago(n.created_at, Date.now())}` : ""}{n.read_at ? " · read" : ""}</p>
                  </div>
                  {!n.read_at ? <button type="button" className="chip-btn" disabled={busy !== null} onClick={() => void act(n.id, async () => { await api.markRead(me, n.id); await reload(); }, null, "Could not mark that read")}>Mark read</button> : null}
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">{unreadOnly ? "Nothing unread." : "No notifications."}</p>
          )
        }
      </State>
    </Panel>
  );
}

// ---- Preferences ---------------------------------------------------------

function Prefs({ api, me, ws }: { api: ToolsApi; me: string | null; ws: string | null }) {
  const key = me;
  const mode = useLoad(key, () => api.deliveryMode(me ?? ""));
  const email = useLoad(key, () => api.email(me ?? ""));
  const prefs = useLoad(key, () => api.prefs(me ?? ""));
  const channelFollows = useLoad(key, () => api.follows(me ?? "", "channel"));
  const threadFollows = useLoad(key, () => api.follows(me ?? "", "thread"));
  const workspaceChannels = useLoad(ws, () => api.channels(ws ?? ""));
  const [emailDraft, setEmailDraft] = useState("");
  const [name, setName] = useState("");
  const [muteKind, setMuteKind] = useState<string>(NOTIFICATION_KINDS[0]);
  const [follow, setFollow] = useState({ channel: "", thread: "" });
  const { busy, act } = useAct();
  const followedThreadIds = (threadFollows.value ?? []).map((f) => f.thread_id ?? "").filter(Boolean);
  const workspaceTasks = useWorkspaceTasks(ws, api, followedThreadIds);
  const threadMenu = workspaceTasks.status === "ready" ? tasksToFollow(workspaceTasks.tasks, followedThreadIds) : [];
  if (!me) return <Panel title="Preferences">{SELF_ONLY}</Panel>;
  const muted = new Set((prefs.value ?? []).filter((p) => p.muted).map((p) => p.kind));
  return (
    <div className="tool-grid">
      <SessionCard />
      <Panel title="Your name">
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); void act("name", () => api.setDisplayName(me, name.trim()), name.trim() ? "Name saved" : "Name cleared", "Could not save your name"); }}>
          <label className="field grow">Display name<input value={name} onChange={(e) => setName(e.target.value)} placeholder="Leave empty to clear" autoComplete="name" /></label>
          <button className="button steel" disabled={busy !== null}>Save</button>
        </form>
      </Panel>
      <Panel title="How you hear about things">
        <State loaded={mode}>
          {(m) => (
            <div className="seg" role="radiogroup" aria-label="Delivery">
              {(["immediate", "digest"] as const).map((v) => (
                <button key={v} type="button" role="radio" aria-checked={m === v} className={m === v ? "on" : ""} disabled={busy !== null} onClick={() => void act(v, async () => { await api.setDeliveryMode(me, v); await mode.reload(); }, v === "digest" ? "You will get a digest" : "You will hear right away", "Could not change delivery")}>
                  {v === "digest" ? "Digest" : "Right away"}
                </button>
              ))}
            </div>
          )}
        </State>
        <h3>Email</h3>
        <State loaded={email}>{(v) => <p className="muted small">{v ? `Now: ${v}` : "No email set."}</p>}</State>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); void act("email", async () => { await api.setEmail(me, emailDraft); setEmailDraft(""); await email.reload(); }, "Email saved", "Could not save that email"); }}>
          <label className="field grow">New email<input type="email" value={emailDraft} onChange={(e) => setEmailDraft(e.target.value)} autoComplete="email" /></label>
          <button className="button steel" disabled={busy !== null || !emailDraft.trim()}>Save</button>
          {email.value ? <button type="button" className="button ghost" disabled={busy !== null} onClick={() => void act("clear", async () => { await api.clearEmail(me); await email.reload(); }, "Email removed", "Could not remove the email")}>Remove</button> : null}
        </form>
      </Panel>
      <Panel title="Muted kinds">
        <State loaded={prefs}>{() => (muted.size ? <ul className="caps">{[...muted].map((k) => <li key={k}><code>{k}</code></li>)}</ul> : <p className="muted small">Nothing muted.</p>)}</State>
        <div className="row-form">
          <label className="field grow">Kind<select value={muteKind} onChange={(e) => setMuteKind(e.target.value)}>{NOTIFICATION_KINDS.map((k) => <option key={k} value={k}>{k.replace(/_/g, " ")}</option>)}</select></label>
          <button type="button" className="button steel" disabled={busy !== null} onClick={() => void act("mute", async () => { await api.setMute(me, muteKind, !muted.has(muteKind)); await prefs.reload(); }, muted.has(muteKind) ? "Unmuted" : "Muted", "Could not change that")}>{muted.has(muteKind) ? "Unmute" : "Mute"}</button>
        </div>
      </Panel>
      {(["channel", "thread"] as const).map((what) => {
        const loaded = what === "channel" ? channelFollows : threadFollows;
        const known = workspaceChannels.value ?? [];
        const followedIds = (channelFollows.value ?? []).map((f) => f.channel_id ?? "").filter(Boolean);
        const channelMenu = what === "channel" && known.length > 0;
        return (
          <Panel key={what} title={what === "channel" ? "Channels you follow" : "Threads you follow"}>
            <State loaded={loaded}>
              {(rows) =>
                rows.length ? (
                  <ul className="tool-list">
                    {rows.map((f) => {
                      const id = (what === "channel" ? f.channel_id : f.thread_id) ?? "";
                      const channelLabel = what === "channel" ? channelFollowLabel(id, known) : "";
                      const taskLabel = what === "thread" ? taskFollowLabel(id, workspaceTasks.byId) : null;
                      const taskWaiting = Boolean(taskLabel && !taskLabel.resolved && workspaceTasks.status === "loading");
                      return (
                        <li key={id} tabIndex={-1} className="tool-row">
                          {taskWaiting ? (
                            <span className="muted">Loading…</span>
                          ) : taskLabel?.resolved ? (
                            <a className="grow" href={convHash({ kind: "task", id })}>{taskLabel.text}</a>
                          ) : taskLabel ? (
                            <a className="grow" href={convHash({ kind: "task", id })}><code className="mono-id">{id}</code></a>
                          ) : channelLabel.startsWith("#") ? (
                            <span>{channelLabel}</span>
                          ) : (
                            <code className="mono-id">{channelLabel}</code>
                          )}
                          <button type="button" className="chip-btn" disabled={busy !== null} onClick={() => void act(id, async () => { await api.unfollow(me, what, id); await loaded.reload(); }, "Unfollowed", "Could not unfollow")}>Unfollow</button>
                        </li>
                      );
                    })}
                  </ul>
                ) : (
                  <p className="muted small">None.</p>
                )
              }
            </State>
            <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); const id = follow[what].trim(); if (!id) return; void act("f", async () => { await api.follow(me, what, id); setFollow((x) => ({ ...x, [what]: "" })); await loaded.reload(); }, "Following", "Could not follow that"); }}>
              {what === "channel" ? (
                channelMenu ? (
                  <label className="field grow">Channel<select value={follow.channel} onChange={(e) => setFollow((x) => ({ ...x, channel: e.target.value }))}>
                    <option value="">Choose a channel</option>
                    {channelsToFollow(known, followedIds).map((c) => <option key={c.id} value={c.id}>{`#${c.name}`}</option>)}
                  </select></label>
                ) : (
                  <label className="field grow">Channel id<input value={follow.channel} onChange={(e) => setFollow((x) => ({ ...x, channel: e.target.value }))} /></label>
                )
              ) : workspaceTasks.status === "loading" ? (
                <label className="field grow">Task<select disabled defaultValue=""><option value="">Loading…</option></select></label>
              ) : threadMenu.length > 0 ? (
                <label className="field grow">Task<select value={follow.thread} onChange={(e) => setFollow((x) => ({ ...x, thread: e.target.value }))}>
                  <option value="">Choose a task</option>
                  {threadMenu.map((task) => <option key={task.id} value={task.id}>{task.label}</option>)}
                </select></label>
              ) : (
                <label className="field grow">Thread id<input value={follow.thread} onChange={(e) => setFollow((x) => ({ ...x, thread: e.target.value }))} /></label>
              )}
              <button className="button steel" disabled={busy !== null || !follow[what].trim()}>Follow</button>
            </form>
          </Panel>
        );
      })}
    </div>
  );
}

// ---- Deliveries ----------------------------------------------------------

function Deliveries({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const [kind, setKind] = useState<string>("all");
  const [status, setStatus] = useState<DeliveryStatus>("quarantined");
  const list = useLoad(ws ? `${ws}|${kind}|${status}` : null, () => api.deliveries(ws ?? "", kind, status));
  const { busy, act } = useAct();
  return (
    <Panel title="Deliveries">
      <div className="row-form">
        <label className="field">Kind<select value={kind} onChange={(e) => setKind(e.target.value)}>{DELIVERY_KINDS.map((k) => <option key={k} value={k}>{k}</option>)}</select></label>
        <div className="seg" role="radiogroup" aria-label="Status">
          {(["quarantined", "pending", "delivered"] as const).map((s) => (
            <button key={s} type="button" role="radio" aria-checked={status === s} className={status === s ? "on" : ""} onClick={() => setStatus(s)}>{s === "quarantined" ? "Dead letters" : s === "pending" ? "All" : "Delivered"}</button>
          ))}
        </div>
      </div>
      <State loaded={list}>
        {(rows) =>
          rows.length ? (
            <ul className="tool-list">
              {rows.map((d) => {
                const st = deliveryState(d);
                return (
                  <li key={`${d.kind}-${d.id}`} tabIndex={-1} className="tool-row">
                    <div className="grow">
                      <div className="token-head"><b>{d.kind} #{d.id}</b><span className={`state-chip ${st === "dlq" ? "revoked" : st === "delivered" ? "active" : ""}`}>{st === "dlq" ? "Dead letter" : st === "delivered" ? "Delivered" : "Pending"}</span></div>
                      <p className="small break">{d.target_url}</p>
                      {d.last_error ? <p className="err small">{d.last_error}</p> : null}
                      <p className="muted small">{d.attempts} attempt{d.attempts === 1 ? "" : "s"}{d.created_at ? ` · made ${ago(d.created_at, Date.now())}` : ""}{d.next_attempt_at ? ` · next ${new Date(d.next_attempt_at).toLocaleString()}` : ""}</p>
                    </div>
                    <button type="button" className="chip-btn" disabled={busy !== null || !ws} onClick={() => void act(String(d.id), async () => { await api.replay(ws ?? "", d); await list.reload(); }, `Replayed ${d.kind} #${d.id}`, "Could not replay that")}>Replay</button>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="muted">No deliveries here.</p>
          )
        }
      </State>
    </Panel>
  );
}

// ---- Slash commands ------------------------------------------------------

function Commands({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const list = useLoad(ws, () => api.commands(ws ?? ""));
  const [form, setForm] = useState({ name: "", description: "", handlerKind: HANDLER_KINDS[0] as string, handlerTarget: "" });
  const [secret, setSecret] = useState<{ name: string; value: string } | null>(null);
  const { busy, act } = useAct();
  const submit = (e: FormEvent) => {
    e.preventDefault();
    void act("reg", async () => {
      const out = await api.register(ws ?? "", form);
      setSecret(out.secret ? { name: form.name.replace(/^\//, ""), value: out.secret } : null);
      setForm((f) => ({ ...f, name: "", description: "", handlerTarget: "" }));
      await list.reload();
    }, `Registered /${form.name.replace(/^\//, "")}`, "Could not register that command");
  };
  return (
    <div className="tool-grid">
      <Panel title="Slash commands">
        <State loaded={list}>
          {(rows) =>
            rows.length ? (
              <ul className="tool-list">
                {rows.map((c) => (
                  <li key={c.id} tabIndex={-1} className={`tool-row${c.enabled ? "" : " read"}`}>
                    <div className="grow">
                      <div className="token-head"><b>/{c.name}</b><span className={`state-chip ${c.enabled ? "active" : "revoked"}`}>{c.enabled ? c.handler_kind : "Revoked"}</span></div>
                      {c.description ? <p className="small">{c.description}</p> : null}
                      <p className="muted small break">→ {c.handler_target}</p>
                    </div>
                    {c.enabled ? <button type="button" className="chip-btn" disabled={busy !== null} onClick={() => void act(c.id, async () => { await api.revokeCommand(ws ?? "", c.id); await list.reload(); }, `Revoked /${c.name}`, "Could not revoke that")}>Revoke</button> : null}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">No slash commands yet.</p>
            )
          }
        </State>
      </Panel>
      <Panel title="Register a command">
        {secret ? <SecretOnce title={`Signing secret for /${secret.name}`} secret={secret.value} onDone={() => setSecret(null)} /> : null}
        <form className="stack-form" onSubmit={submit}>
          <label className="field">Name<input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="deploy" autoCapitalize="off" spellCheck={false} /></label>
          <label className="field">Handler<select value={form.handlerKind} onChange={(e) => setForm({ ...form, handlerKind: e.target.value })}>{HANDLER_KINDS.map((k) => <option key={k} value={k}>{k === "http" ? "HTTP endpoint" : "MCP tool"}</option>)}</select></label>
          <label className="field">{form.handlerKind === "http" ? "URL" : "Tool name"}<input value={form.handlerTarget} onChange={(e) => setForm({ ...form, handlerTarget: e.target.value })} autoCapitalize="off" spellCheck={false} /></label>
          <label className="field">Description<input value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} placeholder="Optional" /></label>
          <button className="button steel" disabled={busy !== null || !ws}>Register</button>
        </form>
      </Panel>
    </div>
  );
}

// ---- Explore -------------------------------------------------------------

function Explore({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const channels = useLoad(ws, () => api.channels(ws ?? ""));
  const [channel, setChannel] = useState("");
  const cid = channel || channels.value?.[0]?.id || "";
  const queue = useLoad(cid ? cid : null, async () => ({ depth: await api.depth(cid), occ: await api.occupancy(cid) }));
  const schedules = useLoad(ws, () => api.schedules(ws ?? ""));
  const [kind, setKind] = useState<string>(EVENT_KINDS[0]);
  const events = useLoad(ws ? `${ws}|${kind}` : null, () => api.events(ws ?? "", kind));
  const threadIds = (events.value ?? []).flatMap((event) => (event.thread_id ? [event.thread_id] : []));
  const tasks = useWorkspaceTasks(ws, api, threadIds);
  const { session } = useSession();
  const federation = holds(session.grant, FEDERATION_ADMIN);
  const peers = useLoad(ws && federation ? ws : null, () => api.peers(ws ?? ""));
  return (
    <div className="tool-grid">
      <Panel title="Queue">
        <State loaded={channels}>
          {(rows) => (
            <label className="field">Channel<select value={cid} onChange={(e) => setChannel(e.target.value)}>{rows.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}</select></label>
          )}
        </State>
        {cid ? (
          <State loaded={queue}>
            {({ depth, occ }) => (
              <dl className="facts">
                <div><dt>Open</dt><dd>{depth.open}</dd></div>
                <div><dt>Ready</dt><dd>{depth.ready}</dd></div>
                <div><dt>Assigned</dt><dd>{depth.assigned}</dd></div>
                <div><dt>Blocked</dt><dd>{depth.blocked}</dd></div>
                <div><dt>No one can claim</dt><dd>{depth.unclaimable}</dd></div>
                {occ ? <div><dt>Working now</dt><dd>{occ.working} working · {occ.claimed} claimed · {occ.queued} queued</dd></div> : null}
              </dl>
            )}
          </State>
        ) : null}
      </Panel>
      <Panel title="Schedules">
        <State loaded={schedules}>
          {(rows) =>
            rows.length ? (
              <ul className="tool-list">
                {rows.map((s, i) => (
                  <li key={s.id ?? i} tabIndex={-1} className={`tool-row${s.active ? "" : " read"}`}>
                    <div className="grow"><b>{s.title}</b><p className="muted small">{scheduleCadence(s)} · {s.active ? `next ${new Date(s.next_run_at).toLocaleString()}` : "paused"}</p></div>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">No schedules.</p>
            )
          }
        </State>
      </Panel>
      <Panel title="Recent events">
        <label className="field">Kind<select value={kind} onChange={(e) => setKind(e.target.value)}>{EVENT_KINDS.map((k) => <option key={k} value={k}>{k.replace(/_/g, " ")}</option>)}</select></label>
        <State loaded={events}>
          {(rows) =>
            rows.length ? (
              <ul className="tool-list">
                {rows.map((e) => (
                  <li key={e.id} tabIndex={-1} className="tool-row"><span className="mono-id">#{e.id}</span><span className="grow small">{eventTarget(e, tasks.byId, channels.value ?? [])}</span></li>
                ))}
              </ul>
            ) : (
              <p className="muted">No recent {kind.replace(/_/g, " ")} events.</p>
            )
          }
        </State>
      </Panel>
      {federation ? (
        <Panel title="Peers">
          <State loaded={peers}>
            {(read) => (!read.allowed ? <p className="muted small">{PEERS_NEED}</p> : read.value.length ? <ul className="tool-list">{read.value.map((p) => <li key={p.id} tabIndex={-1} className="tool-row"><b>{p.name ?? p.id}</b><span className="muted small break">{p.base_url ?? ""}</span></li>)}</ul> : <p className="muted">No peers.</p>)}
          </State>
        </Panel>
      ) : null}
    </div>
  );
}

// ---- Operator ------------------------------------------------------------

/**
 * Rebuild search embeddings. A workspace's own rebuild needs workspace:write;
 * every workspace at once is an operator's call (operator:global).
 */
function Reindex({ api, ws, server }: { api: ToolsApi; ws: string | null; server: boolean }) {
  const [job, setJob] = useState<ReindexJob | null>(null);
  const [jobId, setJobId] = useState("");
  const { busy, act } = useAct();
  const start = (global: boolean) =>
    void act(global ? "g" : "w", async () => {
      const j = await api.reindex(global ? null : ws);
      setJob(j);
      setJobId(j.job_id);
    }, "Reindex started", "Could not start the reindex");
  return (
      <Panel title={server ? "Reindex embeddings" : "Search index"}>
        <p className="muted small">{server ? "Rebuilds search embeddings for this workspace, or for every workspace on the server." : "Rebuilds this workspace's search embeddings, if search results look stale."}</p>
        <div className="row-form">
          <button type="button" className="button steel" disabled={busy !== null || !ws} onClick={() => start(false)}>{server ? "This workspace" : "Rebuild"}</button>
          {server ? <button type="button" className="button ghost" disabled={busy !== null} onClick={() => start(true)}>Whole server</button> : null}
        </div>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); if (!jobId.trim()) return; void act("poll", async () => setJob(await api.reindexJob(jobId)), null, "Could not check that job"); }}>
          <label className="field grow">Job id<input value={jobId} onChange={(e) => setJobId(e.target.value)} spellCheck={false} /></label>
          <button className="button ghost" disabled={busy !== null || !jobId.trim()}>Check</button>
        </form>
        {job ? (
          <dl className="facts">
            <div><dt>Job</dt><dd className="mono-id">{job.job_id}</dd></div>
            <div><dt>Status</dt><dd>{job.status}</dd></div>
            <div><dt>Scope</dt><dd>{job.workspace_id ? "This workspace" : "Whole server"}</dd></div>
            <div><dt>Model</dt><dd>{job.embedding_model}</dd></div>
            {job.processed !== undefined ? <div><dt>Done</dt><dd>{job.processed}{job.failed ? ` · ${job.failed} failed` : ""}</dd></div> : null}
            {job.error ? <div><dt>Error</dt><dd className="err">{job.error}</dd></div> : null}
          </dl>
        ) : null}
      </Panel>
  );
}

function Operator({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const { session } = useSession();
  const [limit, setLimit] = useState(100);
  const [auditOn, setAuditOn] = useState(false);
  const audit = useLoad(auditOn ? String(limit) : null, () => api.audit(limit));
  return (
    <div className="tool-grid">
      {holds(session.grant, OPERATOR_GLOBAL) ? <Reindex api={api} ws={ws} server /> : null}
      {holds(session.grant, AUDIT_READ_GLOBAL) ? (
      <Panel title="Server audit" action={<button type="button" className="chip-btn" onClick={() => (auditOn ? void audit.reload() : setAuditOn(true))}>{auditOn ? "Refresh" : "Load"}</button>}>
        <p className="muted small">Every workspace's audit trail. Needs a token with audit:read-global.</p>
        <label className="field">Rows<select value={limit} onChange={(e) => setLimit(Number(e.target.value))}>{[50, 100, 200, 500].map((n) => <option key={n} value={n}>{n}</option>)}</select></label>
        {auditOn ? (
          <State loaded={audit}>
            {(rows) =>
              rows.length ? (
                <ul className="tool-list">
                  {rows.map((e, i) => (
                    <li key={i} tabIndex={-1} className="tool-row">
                      <div className="grow"><b>{e.action}</b><p className="muted small">{new Date(e.occurred_at).toLocaleString()} · {e.actor_id ? `by ${e.actor_id.slice(0, 8)}` : "system"}{e.target_kind ? ` · ${e.target_kind}${e.target_id ? ` ${e.target_id.slice(0, 8)}` : ""}` : ""}</p></div>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted">No audit events.</p>
              )
            }
          </State>
        ) : null}
      </Panel>
      ) : null}
    </div>
  );
}

// ---- Recent notices (feedback) -------------------------------------------

function Recent() {
  const list = useSyncExternalStore(notices.subscribe, notices.snapshot);
  return (
    <Panel title="Recent" action={list.length ? <button type="button" className="chip-btn" onClick={() => notices.clear()}>Clear</button> : undefined}>
      {list.length ? (
        <ul className="tool-list">
          {list.map((n) => (
            <li key={n.id} tabIndex={-1} className={`tool-row notice ${n.tone}`}>
              <span className="grow">{n.text}</span>
              <span className="muted small">{new Date(n.at).toLocaleTimeString()}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">Nothing yet. What you do here, and what goes wrong, shows up in this list.</p>
      )}
    </Panel>
  );
}

// ---- Search --------------------------------------------------------------

function Search({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const people = usePeopleApi();
  const members = useLoad(ws, () => people.people(ws ?? ""));
  const [q, setQ] = useState("");
  const [mode, setMode] = useState<string>(SEARCH_MODES[0]);
  const [author, setAuthor] = useState("");
  const [ran, setRan] = useState<string | null>(null);
  const hits = useLoad(ws && ran ? ran : null, () => api.search(ws ?? "", ran ?? ""));
  const threadIds = (hits.value ?? []).flatMap((hit) => (hit.thread_id ? [hit.thread_id] : []));
  const tasks = useWorkspaceTasks(ws, api, threadIds);
  if (!ws) return <Panel title="Search">{SELF_ONLY}</Panel>;
  const authorReady = members.value != null;
  return (
    <div className="tool-grid">
      <Panel title="Search messages">
        <form className="row-form" role="search" onSubmit={(e: FormEvent) => { e.preventDefault(); if (q.trim()) setRan(searchQuery(q, mode, "", author)); }}>
          <label className="field grow">Words<input type="search" value={q} onChange={(e) => setQ(e.target.value)} /></label>
          <label className="field">Mode<select value={mode} onChange={(e) => setMode(e.target.value)}>{SEARCH_MODES.map((m) => <option key={m} value={m}>{m}</option>)}</select></label>
          {members.status === "error" && !authorReady ? (
            <label className="field">Author id<input value={author} onChange={(e) => setAuthor(e.target.value)} spellCheck={false} /></label>
          ) : (
            <label className="field">Author<select value={authorReady ? author : ""} onChange={(e) => setAuthor(e.target.value)} disabled={!authorReady}>
              {(authorReady ? memberChoices(members.value ?? []) : [{ id: "", label: "Loading…" }]).map((c) => <option key={c.id || "anyone"} value={c.id}>{c.label}</option>)}
            </select></label>
          )}
          <button className="button steel" disabled={!q.trim()}>Search</button>
        </form>
        {ran ? (
          <State loaded={hits}>
            {(rows) =>
              rows.length ? (
                <ul className="tool-list">
                  {rows.map((h, i) => (
                    <li key={h.message_id ?? h.id ?? i} tabIndex={-1} className="tool-row">
                      <div className="grow">
                        <p>{h.snippet || h.body || "(no text)"}</p>
                        {h.thread_id ? <p className="muted small"><a className="muted small" href={convHash({ kind: "task", id: h.thread_id })}>{threadLinkLabel(h.thread_id, tasks.byId, "Open the thread")}</a>{memberBy(members.value, h.author_id)}</p> : null}
                      </div>
                    </li>
                  ))}
                </ul>
              ) : <p className="muted">No messages match.</p>
            }
          </State>
        ) : null}
      </Panel>
    </div>
  );
}

// ---- Workspace -----------------------------------------------------------

function Workspace({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const { session } = useSession();
  const { busy, act } = useAct();
  const [name, setName] = useState("");
  const [channel, setChannel] = useState("");
  const [task, setTask] = useState({ channel: "", title: "" });
  const [peer, setPeer] = useState({ name: "", url: "" });
  const [agent, setAgent] = useState({ handle: "", display: "" });
  const [dropping, setDropping] = useState<string | null>(null);
  const [auditOn, setAuditOn] = useState(false);
  const channels = useLoad(ws, () => api.channels(ws ?? ""));
  const federation = holds(session.grant, FEDERATION_ADMIN);
  const peers = useLoad(ws && federation ? ws : null, () => api.peers(ws ?? ""));
  const installs = useLoad(ws, () => api.appInstallations(ws ?? ""));
  const audit = useLoad(ws && auditOn ? ws : null, () => api.workspaceAudit(ws ?? "", 100));
  const people = usePeopleApi();
  const auditMembers = useLoad(ws && auditOn ? ws : null, () => people.people(ws ?? ""));
  const spaces = useLoad("switchable", () => api.switchable());
  if (!ws) return <Panel title="Workspace">{SELF_ONLY}</Panel>;
  const base = session.status === "signed-in" ? session.credentials.base : "";
  const cid = task.channel || channels.value?.[0]?.id || "";
  const peerWhy = peerRefusal(peer.name, peer.url);
  const agentWhy = agent.handle ? handleRefusal(agent.handle) : HANDLE_REQUIRED;
  return (
    <div className="tool-grid">
      <Panel title="Name">
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); void act("rename", () => api.renameWorkspace(ws, name), "Workspace renamed", "Could not rename the workspace"); }}>
          <label className="field grow">New name<input value={name} onChange={(e) => setName(e.target.value)} maxLength={120} /></label>
          <button className="button steel" disabled={busy !== null || !name.trim()}>Rename</button>
        </form>
        {spaces.value && spaces.value.length > 1 ? (
          <>
            <h3 className="small">Switch workspace</h3>
            <ul className="tool-list">
              {spaces.value.map((w) => (
                <li key={w.workspace_id} tabIndex={-1} className="tool-row">
                  <div className="grow"><b>{w.name}</b></div>
                  {w.current ? <span className="state-chip">Current</span> : <a className="chip-btn" href={switchHref(base, w.workspace_id)}>Switch</a>}
                </li>
              ))}
            </ul>
          </>
        ) : null}
      </Panel>
      <Panel title="Channels and tasks">
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); void act("chan", async () => { await api.createChannel(ws, channel); setChannel(""); await channels.reload(); }, "Channel created", "Could not create the channel"); }}>
          <label className="field grow">New channel<input value={channel} onChange={(e) => setChannel(e.target.value)} /></label>
          <button className="button steel" disabled={busy !== null || !channel.trim()}>Create</button>
        </form>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); void act("task", async () => { await api.createThread(cid, task.title); setTask({ ...task, title: "" }); }, "Task created", "Could not create the task"); }}>
          <label className="field">In<select value={cid} onChange={(e) => setTask({ ...task, channel: e.target.value })}>{(channels.value ?? []).map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}</select></label>
          <label className="field grow">New task<input value={task.title} onChange={(e) => setTask({ ...task, title: e.target.value })} /></label>
          <button className="button steel" disabled={busy !== null || !cid || !task.title.trim()}>Create</button>
        </form>
      </Panel>
      <Panel title="Agents">
        <p className="muted small">Adds an agent member. Needs a pasted token with token:admin. Mint its token from People afterwards.</p>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); if (agentWhy) return; void act("agent", async () => { await api.createAgent(ws, agent.handle, agent.display); setAgent({ handle: "", display: "" }); }, "Agent added", "Could not add the agent. It may exist already: check People before trying again"); }}>
          <label className="field">Handle<input value={agent.handle} onChange={(e) => setAgent({ ...agent, handle: e.target.value })} spellCheck={false} autoCapitalize="none" /></label>
          <label className="field grow">Display name<input value={agent.display} onChange={(e) => setAgent({ ...agent, display: e.target.value })} /></label>
          <button className="button steel" disabled={busy !== null || agentWhy !== null}>Add</button>
        </form>
        {agent.handle && agentWhy ? <p className="err small">{agentWhy}</p> : null}
      </Panel>
      {holds(session.grant, "workspace:write") ? <Reindex api={api} ws={ws} server={false} /> : null}
      {federation ? (
      <Panel title="Peers">
        <State loaded={peers}>
          {(read) =>
            !read.allowed ? (
              <p className="muted small">{PEERS_NEED}</p>
            ) : read.value.length ? (
              <ul className="tool-list">
                {read.value.map((p) => (
                  <li key={p.id} tabIndex={-1} className="tool-row">
                    <div className="grow"><b>{p.name || p.id}</b><p className="muted small">{p.base_url}</p></div>
                    {dropping === p.id ? (
                      <>
                        <button type="button" className="chip-btn danger" disabled={busy !== null} onClick={() => void act("drop", async () => { await api.removePeer(ws, p.id); setDropping(null); await peers.reload(); }, "Peer removed", "Could not remove the peer")}>Remove</button>
                        <button type="button" className="chip-btn" onClick={() => setDropping(null)}>Keep</button>
                      </>
                    ) : <button type="button" className="chip-btn" onClick={() => setDropping(p.id)}>Remove…</button>}
                  </li>
                ))}
              </ul>
            ) : <p className="muted">No peers yet.</p>
          }
        </State>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); if (peerWhy) return; void act("peer", async () => { await api.addPeer(ws, peer.name, peer.url); setPeer({ name: "", url: "" }); await peers.reload(); }, "Peer added", "Could not add the peer"); }}>
          <label className="field">Name<input value={peer.name} onChange={(e) => setPeer({ ...peer, name: e.target.value })} /></label>
          <label className="field grow">Base URL<input type="url" value={peer.url} onChange={(e) => setPeer({ ...peer, url: e.target.value })} spellCheck={false} /></label>
          <button className="button steel" disabled={busy !== null || peerWhy !== null}>Add</button>
        </form>
      </Panel>
      ) : null}
      {/* Installing an app happens elsewhere; an empty list here is only noise. */}
      {installs.value?.length ? (
      <Panel title="App installations">
        <State loaded={installs}>
          {(rows) =>
            rows.length ? (
              <ul className="tool-list">
                {rows.map((r) => (
                  <li key={r.id} tabIndex={-1} className={`tool-row${r.revoked_at ? " read" : ""}`}>
                    <div className="grow"><b>{r.app_id}</b><p className="muted small">Install {r.id}{r.revoked_at ? " · revoked" : ""}</p></div>
                  </li>
                ))}
              </ul>
            ) : <p className="muted">No apps installed.</p>
          }
        </State>
      </Panel>
      ) : null}
      <Panel title="Workspace audit" action={<button type="button" className="chip-btn" onClick={() => (auditOn ? void audit.reload() : setAuditOn(true))}>{auditOn ? "Refresh" : "Load"}</button>}>
        <p className="muted small">The last 100 audited actions in this workspace.</p>
        {auditOn ? (
          <State loaded={audit}>
            {(rows) =>
              rows.length ? (
                <ul className="tool-list">
                  {rows.map((a, i) => {
                    const who = a.actor_id ? auditActor(a.actor_id, auditMembers) : null;
                    return (
                      <li key={i} tabIndex={-1} className="tool-row">
                        <div className="grow"><b>{a.action}</b><p className="muted small">{new Date(a.occurred_at).toLocaleString()}{who ? ` \u00b7 by ${who}` : ""}{a.target_kind ? ` · ${a.target_kind}` : ""}</p></div>
                      </li>
                    );
                  })}
                </ul>
              ) : <p className="muted">Nothing audited yet.</p>
            }
          </State>
        ) : null}
      </Panel>
    </div>
  );
}

/** Dark, Light or System for this browser. Dark until someone picks. */
function Appearance() {
  const [choice, choose] = useTheme();
  // A radio group is one tab stop; the arrow keys move the choice, as they do
  // in a native one.
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const at = THEME_CHOICES.findIndex((c) => c.id === choice);
    const next = THEME_CHOICES[(at + step + THEME_CHOICES.length) % THEME_CHOICES.length];
    choose(next.id);
    e.currentTarget.querySelector<HTMLButtonElement>(`[data-choice="${next.id}"]`)?.focus();
  };
  return (
    <Panel title="Appearance">
      <p className="muted small" id="theme-why">Kept in this browser. System follows your device's light or dark setting.</p>
      {/* eslint-disable-next-line jsx-a11y/interactive-supports-focus -- a roving tabindex: the checked radio takes focus and the arrows move it. */}
      <div className="seg theme-pick" role="radiogroup" aria-label="Theme" aria-describedby="theme-why" onKeyDown={onKey}>
        {THEME_CHOICES.map((c) => (
          <button key={c.id} type="button" role="radio" data-choice={c.id} aria-checked={choice === c.id} tabIndex={choice === c.id ? 0 : -1} className={choice === c.id ? "on" : undefined} onClick={() => choose(c.id)}>
            {c.label}
          </button>
        ))}
      </div>
    </Panel>
  );
}

function Device({ api, me }: { api: ToolsApi; me: string | null }) {
  const install = useSyncExternalStore(installStore.subscribe, installStore.getSnapshot);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const permission = typeof Notification === "undefined" ? "unsupported" : Notification.permission;
  const deps = {
    permission: () => (typeof Notification === "undefined" ? "denied" : Notification.permission),
    requestPermission: () => Notification.requestPermission(),
    pushManager: consolePushManager,
    vapidKey: () => api.vapidKey(),
    save: (m: string, sub: { endpoint: string; keys: Record<string, string> }) => api.savePush(m, sub),
  };
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  useEffect(() => {
    let live = true;
    api.vapidKey().then((k) => live && setHasKey(Boolean(k?.public_key)), () => live && setHasKey(false));
    return () => {
      live = false;
    };
  }, [api]);
  const [subscribed, setSubscribed] = useState(false);
  const [checked, setChecked] = useState(0);
  useEffect(() => {
    let live = true;
    pushSubscribed(consolePushManager).then((on) => live && setSubscribed(on));
    return () => {
      live = false;
    };
  }, [checked]);
  const onWhy = subscribed ? null : pushRefusal({ me, permission, hasKey });
  const run = async (fn: () => Promise<PushOutcome>) => {
    setBusy(true);
    try {
      const out = await fn();
      setNote(PUSH_WORDS[out.reason] ?? out.reason);
    } finally {
      setBusy(false);
      setChecked((n) => n + 1);
    }
  };
  return (
    <>
      <Appearance />
      <Panel title="Install">
        {install.installed ? (
          <p className="muted">The console is installed on this device.</p>
        ) : install.canPrompt ? (
          <button type="button" className="steel" onClick={() => void promptInstall()}>Install the console</button>
        ) : (
          <p className="muted">To install, use your browser's Install or Add to Home Screen option.</p>
        )}
      </Panel>
      <Panel title="Notifications">
        <p className="muted small">Get a notification on this device when a task needs you or someone mentions you.</p>
        {onWhy ? <p className="muted small" id="push-why">{onWhy}</p> : <p className="muted small">Browser permission: {permission}.</p>}
        {subscribed ? <p className="small">Notifications are on for this device.</p> : null}
        <div className="push-actions">
          {subscribed ? (
            <button type="button" className="button ghost" disabled={busy} onClick={() => void run(() => disablePush(deps, (ep) => (me ? api.forgetPush(me, ep) : Promise.resolve())))}>Turn off</button>
          ) : (
            <button type="button" className="button ink" disabled={busy || onWhy !== null} aria-describedby={onWhy ? "push-why" : undefined} onClick={() => void run(() => enablePush(me, deps))}>Turn on</button>
          )}
        </div>
        {note ? <p className="small" role="status">{note}</p> : null}
      </Panel>
    </>
  );
}

// ---- Session ---------------------------------------------------------------

/** What this credential can do, and the way out.
 * A signed-in tab renders this before GET /me resolves the mode: Sign out
 * has to be there in that gap, or a reload can hide the only way out. */
function SessionCard() {
  const { session, store } = useSession();
  const [leaving, setLeaving] = useState(false);
  if (session.status !== "signed-in") return null;
  const signOut = async () => {
    if (leaving) return;
    setLeaving(true);
    await store.logout();
    setLeaving(false);
  };
  const card = session.mode && session.grant ? capabilityCard(session.mode, session.grant) : null;
  return (
    <Panel title="Session">
      <dl className="facts">
        <div><dt>Acting with</dt><dd className="session-credential">{session.mode ? MODE_LINE[session.mode] : "Checking…"}</dd></div>
        <div><dt>Member id</dt><dd className="mono-id session-member">{session.memberId}</dd></div>
        <div><dt>Workspace id</dt><dd className="mono-id">{session.workspaceId}</dd></div>
      </dl>
      {card ? (
        <div className="cap-card">
          <div>
            <h3 className="small">Can <span className="muted cap-count">({card.can.length})</span></h3>
            <ul className="caps cap-can" aria-label="Can">{card.can.map((c) => <li key={c}><code>{c}</code></li>)}</ul>
          </div>
          <div>
            <h3 className="small">Can't <span className="muted">({card.cannot.length})</span></h3>
            <ul className="caps cap-cant" aria-label="Can't">{card.cannot.map((c) => <li key={c}><code>{c}</code></li>)}</ul>
          </div>
        </div>
      ) : null}
      <button type="button" className="button ghost session-signout" disabled={leaving} onClick={() => void signOut()}>Sign out</button>
    </Panel>
  );
}
