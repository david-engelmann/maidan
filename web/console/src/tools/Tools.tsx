// Tools: what the classic board kept on its Tools tabs, minus the gates
// Needs you already answers. Inbox and Preferences are the signed-in
// member's own; Deliveries and Commands are the workspace's; Explore reads
// queues, schedules, events and peers; Operator is audit and reindexing;
// Search finds messages; Workspace renames, switches, and adds channels,
// tasks, agents and peers, and reads apps and the workspace audit.
// Recent is every notice this tab has shown.
import { type FormEvent, type ReactNode, useState, useSyncExternalStore } from "react";
import { ago } from "../core/board.ts";
import { convHash } from "../core/threads.ts";
import { notices, say, sayError } from "../core/notices.ts";
import {
  DELIVERY_KINDS,
  EVENT_KINDS,
  HANDLER_KINDS,
  HANDLE_REQUIRED,
  SEARCH_MODES,
  handleRefusal,
  peerRefusal,
  searchQuery,
  switchHref,
  NOTIFICATION_KINDS,
  deliveryState,
  scheduleCadence,
  type DeliveryStatus,
  type ReindexJob,
  type ToolsApi,
} from "../core/tools.ts";
import type { Loaded } from "../hooks/usePeople.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";
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
  if (session.status === "signed-out") {
    return (
      <div className="card">
        <h2>Tools</h2>
        <p className="muted">Sign in on the classic board first, then come back.</p>
        <a className="button steel" href="/ui/">Open the classic board</a>
      </div>
    );
  }
  const me = session.status === "signed-in" ? session.memberId : null;
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  return (
    <div className="tools">
      <div className="seg seg-scroll" role="tablist" aria-label="Tools">
        {TOOL_TABS.map((t) => (
          <a key={t.id} role="tab" aria-selected={tab === t.id} href={`#/tools/${t.id}`}>{t.title}</a>
        ))}
      </div>
      {tab === "inbox" ? <Inbox api={api} me={me} /> : null}
      {tab === "prefs" ? <Prefs api={api} me={me} /> : null}
      {tab === "deliveries" ? <Deliveries api={api} ws={ws} /> : null}
      {tab === "commands" ? <Commands api={api} ws={ws} /> : null}
      {tab === "search" ? <Search api={api} ws={ws} /> : null}
      {tab === "workspace" ? <Workspace api={api} ws={ws} /> : null}
      {tab === "explore" ? <Explore api={api} ws={ws} /> : null}
      {tab === "operator" ? <Operator api={api} ws={ws} /> : null}
      {tab === "recent" ? <Recent /> : null}
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

function State<T>({ loaded, empty, children }: { loaded: Loaded<T>; empty?: (v: T) => boolean; children: (v: T) => ReactNode }) {
  if (loaded.status === "loading") return <p className="muted">Loading…</p>;
  if (loaded.value === null) return <p className="err" role="alert">{loaded.error}</p>;
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

// ---- Inbox ---------------------------------------------------------------

function Inbox({ api, me }: { api: ToolsApi; me: string | null }) {
  const [unreadOnly, setUnreadOnly] = useState(true);
  const list = useLoad(me ? `${me}|${unreadOnly}` : null, () => api.notifications(me ?? "", unreadOnly));
  const count = useLoad(me ? `c|${me}|${list.value?.length ?? -1}` : null, () => api.unread(me ?? ""));
  const { busy, act } = useAct();
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
                <li key={n.id} className={`tool-row${n.read_at ? " read" : ""}`}>
                  <div>
                    <b>{n.kind.replace(/_/g, " ")}</b>
                    <p className="muted small">{n.thread_id ? <a href={`#/threads/t/${encodeURIComponent(n.thread_id)}`}>Open the thread</a> : "Workspace"}{n.created_at ? ` · ${ago(n.created_at, Date.now())}` : ""}{n.read_at ? " · read" : ""}</p>
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

function Prefs({ api, me }: { api: ToolsApi; me: string | null }) {
  const key = me;
  const mode = useLoad(key, () => api.deliveryMode(me ?? ""));
  const email = useLoad(key, () => api.email(me ?? ""));
  const prefs = useLoad(key, () => api.prefs(me ?? ""));
  const channels = useLoad(key, () => api.follows(me ?? "", "channel"));
  const threads = useLoad(key, () => api.follows(me ?? "", "thread"));
  const [emailDraft, setEmailDraft] = useState("");
  const [name, setName] = useState("");
  const [muteKind, setMuteKind] = useState<string>(NOTIFICATION_KINDS[0]);
  const [follow, setFollow] = useState({ channel: "", thread: "" });
  const { busy, act } = useAct();
  if (!me) return <Panel title="Preferences">{SELF_ONLY}</Panel>;
  const muted = new Set((prefs.value ?? []).filter((p) => p.muted).map((p) => p.kind));
  return (
    <div className="tool-grid">
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
        const loaded = what === "channel" ? channels : threads;
        return (
          <Panel key={what} title={what === "channel" ? "Channels you follow" : "Threads you follow"}>
            <State loaded={loaded}>
              {(rows) =>
                rows.length ? (
                  <ul className="tool-list">
                    {rows.map((f) => {
                      const id = (what === "channel" ? f.channel_id : f.thread_id) ?? "";
                      return (
                        <li key={id} className="tool-row">
                          <code className="mono-id">{id}</code>
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
              <label className="field grow">{what === "channel" ? "Channel id" : "Thread id"}<input value={follow[what]} onChange={(e) => setFollow((x) => ({ ...x, [what]: e.target.value }))} /></label>
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
                  <li key={`${d.kind}-${d.id}`} className="tool-row">
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
                  <li key={c.id} className={`tool-row${c.enabled ? "" : " read"}`}>
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
  const peers = useLoad(ws, () => api.peers(ws ?? ""));
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
                  <li key={s.id ?? i} className={`tool-row${s.active ? "" : " read"}`}>
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
                  <li key={e.id} className="tool-row"><span className="mono-id">#{e.id}</span><span className="grow small">{e.thread_id ? <a href={`#/threads/t/${encodeURIComponent(e.thread_id)}`}>thread {e.thread_id.slice(0, 8)}</a> : e.channel_id ? `channel ${e.channel_id.slice(0, 8)}` : "workspace"}</span></li>
                ))}
              </ul>
            ) : (
              <p className="muted">No recent {kind.replace(/_/g, " ")} events.</p>
            )
          }
        </State>
      </Panel>
      <Panel title="Peers">
        <State loaded={peers}>
          {(rows) => (rows.length ? <ul className="tool-list">{rows.map((p) => <li key={p.id} className="tool-row"><b>{p.name ?? p.id}</b><span className="muted small break">{p.base_url ?? ""}</span></li>)}</ul> : <p className="muted">No peers.</p>)}
        </State>
      </Panel>
    </div>
  );
}

// ---- Operator ------------------------------------------------------------

function Operator({ api, ws }: { api: ToolsApi; ws: string | null }) {
  const [limit, setLimit] = useState(100);
  const [auditOn, setAuditOn] = useState(false);
  const audit = useLoad(auditOn ? String(limit) : null, () => api.audit(limit));
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
    <div className="tool-grid">
      <Panel title="Reindex embeddings">
        <p className="muted small">Rebuilds search embeddings. The whole server needs a token with token:admin.</p>
        <div className="row-form">
          <button type="button" className="button steel" disabled={busy !== null || !ws} onClick={() => start(false)}>This workspace</button>
          <button type="button" className="button ghost" disabled={busy !== null} onClick={() => start(true)}>Whole server</button>
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
      <Panel title="Server audit" action={<button type="button" className="chip-btn" onClick={() => (auditOn ? void audit.reload() : setAuditOn(true))}>{auditOn ? "Refresh" : "Load"}</button>}>
        <p className="muted small">Every workspace's audit trail. Needs a token with audit:read-global.</p>
        <label className="field">Rows<select value={limit} onChange={(e) => setLimit(Number(e.target.value))}>{[50, 100, 200, 500].map((n) => <option key={n} value={n}>{n}</option>)}</select></label>
        {auditOn ? (
          <State loaded={audit}>
            {(rows) =>
              rows.length ? (
                <ul className="tool-list">
                  {rows.map((e, i) => (
                    <li key={i} className="tool-row">
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
            <li key={n.id} className={`tool-row notice ${n.tone}`}>
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
  const [q, setQ] = useState("");
  const [mode, setMode] = useState<string>(SEARCH_MODES[0]);
  const [author, setAuthor] = useState("");
  const [ran, setRan] = useState<string | null>(null);
  const hits = useLoad(ws && ran ? ran : null, () => api.search(ws ?? "", ran ?? ""));
  if (!ws) return <Panel title="Search">{SELF_ONLY}</Panel>;
  return (
    <div className="tool-grid">
      <Panel title="Search messages">
        <form className="row-form" role="search" onSubmit={(e: FormEvent) => { e.preventDefault(); if (q.trim()) setRan(searchQuery(q, mode, "", author)); }}>
          <label className="field grow">Words<input type="search" value={q} onChange={(e) => setQ(e.target.value)} /></label>
          <label className="field">Mode<select value={mode} onChange={(e) => setMode(e.target.value)}>{SEARCH_MODES.map((m) => <option key={m} value={m}>{m}</option>)}</select></label>
          <label className="field">Author id<input value={author} onChange={(e) => setAuthor(e.target.value)} spellCheck={false} /></label>
          <button className="button steel" disabled={!q.trim()}>Search</button>
        </form>
        {ran ? (
          <State loaded={hits}>
            {(rows) =>
              rows.length ? (
                <ul className="tool-list">
                  {rows.map((h, i) => (
                    <li key={h.message_id ?? h.id ?? i} className="tool-row">
                      <div className="grow">
                        <p>{h.snippet || h.body || "(no text)"}</p>
                        {h.thread_id ? <a className="muted small" href={convHash({ kind: "task", id: h.thread_id })}>Open the thread</a> : null}
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
  const peers = useLoad(ws, () => api.peers(ws ?? ""));
  const installs = useLoad(ws, () => api.appInstallations(ws ?? ""));
  const audit = useLoad(ws && auditOn ? ws : null, () => api.workspaceAudit(ws ?? "", 100));
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
                <li key={w.workspace_id} className="tool-row">
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
      <Panel title="Peers">
        <State loaded={peers}>
          {(rows) =>
            rows.length ? (
              <ul className="tool-list">
                {rows.map((p) => (
                  <li key={p.id} className="tool-row">
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
      <Panel title="App installations">
        <State loaded={installs}>
          {(rows) =>
            rows.length ? (
              <ul className="tool-list">
                {rows.map((r) => (
                  <li key={r.id} className={`tool-row${r.revoked_at ? " read" : ""}`}>
                    <div className="grow"><b>{r.app_id}</b><p className="muted small">Install {r.id}{r.revoked_at ? " · revoked" : ""}</p></div>
                  </li>
                ))}
              </ul>
            ) : <p className="muted">No apps installed.</p>
          }
        </State>
      </Panel>
      <Panel title="Workspace audit" action={<button type="button" className="chip-btn" onClick={() => (auditOn ? void audit.reload() : setAuditOn(true))}>{auditOn ? "Refresh" : "Load"}</button>}>
        <p className="muted small">The last 100 audited actions in this workspace.</p>
        {auditOn ? (
          <State loaded={audit}>
            {(rows) =>
              rows.length ? (
                <ul className="tool-list">
                  {rows.map((a, i) => (
                    <li key={i} className="tool-row">
                      <div className="grow"><b>{a.action}</b><p className="muted small">{new Date(a.occurred_at).toLocaleString()}{a.target_kind ? ` · ${a.target_kind}` : ""}</p></div>
                    </li>
                  ))}
                </ul>
              ) : <p className="muted">Nothing audited yet.</p>
            }
          </State>
        ) : null}
      </Panel>
    </div>
  );
}
