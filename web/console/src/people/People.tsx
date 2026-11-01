// People and Swarm. People is the roster: who is in the workspace, who is
// live, what each one holds, and a member's tokens. Swarm is the builder,
// a steel tile that seats members in the four roles that close the loop,
// and the footprint lists the workers share.
import { type ReactNode, useEffect, useMemo, useRef, useState, useSyncExternalStore, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { BoardApi, ago, leaseLeft } from "../core/board.ts";
import { liveActions } from "../core/bus.ts";
import {
  METER_LABELS,
  NO_SEATS,
  REASON_LABELS,
  ROLES,
  ROLE_NAME,
  PeopleApi,
  circuit,
  circuitStatus,
  filterRoster,
  holding,
  loadSeats,
  nextWell,
  roster,
  saveSeats,
  seat,
  seatRefusal,
  WORKER_PRESET,
  tokenState,
  unseat,
  type Pause,
  type Person,
  type Presence,
  type Role,
  type RosterRow,
  type Seats,
} from "../core/people.ts";
import { hasToken } from "../core/api.ts";
import { say, sayError } from "../core/notices.ts";
import { convHash } from "../core/threads.ts";
import { capsExceeding, parseCaps } from "../core/tools.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";
import { SecretOnce } from "../tools/SecretOnce.tsx";
import { useBoard } from "../hooks/useBoard.ts";
import { usePeople, usePresence, useSwarm, useTokens, type Loaded } from "../hooks/usePeople.ts";
import { useSession } from "../hooks/useSession.ts";

type Tab = "people" | "swarm";

function subscribeHash(fn: () => void): () => void {
  window.addEventListener("hashchange", fn);
  return () => window.removeEventListener("hashchange", fn);
}

/** #/people, #/people/m/{id} and #/people/swarm. */
function useSub(): { tab: Tab; member: string | null } {
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);
  const parts = hash.replace(/^#\/?/, "").split("/");
  if (parts[1] === "swarm") return { tab: "swarm", member: null };
  return { tab: "people", member: parts[1] === "m" && parts[2] ? decodeURIComponent(parts[2]) : null };
}

function initials(name: string): string {
  return name
    .split(/[\s._-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

const PRESENCE_WORD: Record<Presence, string> = { online: "Online", away: "Away", offline: "Offline" };

export function People() {
  const { session, store } = useSession();
  const deps = useMemo(() => {
    const creds = () => store.getSnapshot().credentials;
    const mode = () => store.getSnapshot().mode;
    const f = (url: string, init?: RequestInit) => fetch(url, init);
    return { people: new PeopleApi(f, creds, mode), board: new BoardApi(f, creds, mode) };
  }, [store]);
  const { tab, member } = useSub();
  const { data: people } = usePeople(session, deps.people);
  const { data: board } = useBoard(session, deps.board);
  const presence = usePresence();
  const me = session.status === "signed-in" ? session.memberId : null;
  const held = useMemo(() => holding(board.threads), [board.threads]);
  const rows = useMemo(() => roster(people.value ?? [], presence, held, me), [people.value, presence, held, me]);


  return (
    <div className="people">
      <div className="seg" role="tablist" aria-label="People or swarm">
        <a role="tab" aria-selected={tab === "people"} href="#/people">People</a>
        <a role="tab" aria-selected={tab === "swarm"} href="#/people/swarm">Swarm</a>
      </div>
      {tab === "people" ? (
        <Roster loaded={people} rows={rows} open={member} api={deps.people} />
      ) : (
        <Swarm rows={rows} people={people.value ?? []} api={deps.people} workspace={session.status === "signed-in" ? session.workspaceId : null} presence={presence} />
      )}
    </div>
  );
}

// ---- Roster ----------------------------------------------------------------

function Avatar({ person, presence, large }: { person: Person; presence?: Presence; large?: boolean }) {
  return (
    <span className={`avatar ${person.kind === "human" ? "human" : "agent"}${large ? " lg" : ""}`} aria-hidden="true">
      {initials(person.name) || "?"}
      {presence ? <span className={`pip ${presence}`} /> : null}
    </span>
  );
}

function Roster({ loaded, rows, open, api }: { loaded: Loaded<Person[]>; rows: RosterRow[]; open: string | null; api: PeopleApi }) {
  const [query, setQuery] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const shown = filterRoster(rows, query);
  const live = rows.filter((r) => r.presence !== "offline").length;
  const selected = open ? (rows.find((r) => r.person.id === open) ?? null) : null;

  function setPresence(status: "online" | "away") {
    setNote(liveActions.setPresence(status) ? `You are ${status === "online" ? "online" : "away"}.` : "Not connected yet. Presence goes out once the live connection is back.");
  }

  return (
    <div className={`people-grid${open ? " detail-open" : ""}`}>
      <section className="roster" aria-label="Members">
        <div className="roster-tools">
          <input className="search" type="search" placeholder="Find a member" aria-label="Find a member by name or handle" value={query} onChange={(e) => setQuery(e.target.value)} />
          <div className="presence-set" role="group" aria-label="Your presence">
            <button type="button" className="chip-btn" onClick={() => setPresence("online")}>Online</button>
            <button type="button" className="chip-btn" onClick={() => setPresence("away")}>Away</button>
          </div>
        </div>
        <p className="mono roster-count" aria-live="polite">
          {loaded.status === "ready" ? `${rows.length} members · ${live} live` : loaded.status === "loading" ? "Loading members" : ""}
          {note ? <span className="note"> · {note}</span> : null}
        </p>
        {loaded.error ? <p className={loaded.value ? "stale" : "err"} role="alert">{loaded.error}</p> : null}
        {loaded.status === "ready" && !shown.length ? <p className="muted">{query ? `No member matches “${query}”.` : "No members yet."}</p> : null}
        <ul className="roster-list">
          {shown.map((r) => (
            <li key={r.person.id}>
              <a className={`person-row${open === r.person.id ? " active" : ""}`} href={`#/people/m/${encodeURIComponent(r.person.id)}`} aria-current={open === r.person.id ? "true" : undefined}>
                <Avatar person={r.person} presence={r.presence} />
                <span className="who">
                  <b>
                    {r.person.name}
                    {r.me ? <span className="you"> (you)</span> : null}
                  </b>
                  <small>{r.task ? `${r.task.work_started_at ? "Running" : "Holding"} · ${r.task.title || "untitled"}` : PRESENCE_WORD[r.presence]}</small>
                </span>
                <span className={`kind-badge ${r.person.kind === "human" ? "human" : "agent"}`}>{r.person.kind === "human" ? "Human" : "Agent"}</span>
              </a>
            </li>
          ))}
        </ul>
      </section>
      {open ? (
        <section className="member-detail card" aria-label="Member details">
          <a className="back" href="#/people">People</a>
          {selected ? <MemberDetail key={selected.person.id} row={selected} api={api} /> : loaded.status === "ready" ? <p className="muted">That member is not in this workspace.</p> : <p className="muted">Loading…</p>}
        </section>
      ) : (
        <section className="member-detail card wide-only" aria-hidden="true">
          <p className="muted">Pick a member to see their details and tokens.</p>
        </section>
      )}
    </div>
  );
}

function MemberDetail({ row, api }: { row: RosterRow; api: PeopleApi }) {
  const { session, store } = useSession();
  const [version, setVersion] = useState(0);
  const tokens = useTokens(session, api, row.person.id, version);
  const [now] = useState(() => Date.now());
  // A secret the server just sent. It lives here only: the panel closing,
  // another member opening, or "I have it" drops it. It is never logged.
  const [secret, setSecret] = useState<{ title: string; value: string } | null>(null);
  const [confirm, setConfirm] = useState<{ id: string; op: "rotate" | "revoke" } | null>(null);
  const [busy, setBusy] = useState(false);
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const canWrite = session.status === "signed-in" && hasToken(session.credentials);
  const pageToken = session.status === "signed-in" ? session.credentials.sessionTokenId ?? null : null;
  const tools = useToolsApi();
  const held = useLoad(canWrite ? "me-caps" : null, () => tools.me());
  const p = row.person;
  const run = async (what: () => Promise<void>, done: string, fallback: string) => {
    if (busy) return;
    setBusy(true);
    try {
      await what();
      say(done);
      setVersion((v) => v + 1);
    } catch (e) {
      sayError(e, fallback);
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };
  const tokenAct = (id: string, op: "rotate" | "revoke") => {
    if (!confirm || confirm.id !== id || confirm.op !== op) return setConfirm({ id, op });
    if (op === "revoke") void run(() => api.revoke(id), "Token revoked", "Could not revoke that token");
    else
      void run(async () => {
        const own = id === pageToken;
        const connection = store.connection();
        const out = await api.rotate(id);
        setSecret({ title: "New token. The old one no longer works.", value: out.secret });
        if (own && !(await store.adoptRotated(out.secret, connection))) say("This page's sign-in changed during the rotation, so it was not moved onto the new token.", "error");
      }, "Token rotated", "Could not rotate that token");
  };
  return (
    <div className="detail-body">
      <div className="detail-head">
        <Avatar person={p} presence={row.presence} large />
        <div>
          <h2>{p.name}{row.me ? <span className="you"> (you)</span> : null}</h2>
          <p className="muted">{p.handle ? `@${p.handle} · ` : ""}{p.kind === "human" ? "Human" : "Agent"} · {PRESENCE_WORD[row.presence]}</p>
        </div>
      </div>
      <dl className="facts">
        <div><dt>Member id</dt><dd className="mono-id">{p.id}</dd></div>
        {p.createdAt ? <div><dt>Joined</dt><dd>{ago(p.createdAt, now)}</dd></div> : null}
        <div><dt>Holds</dt><dd>{row.task ? <a href={convHash({ kind: "task", id: row.task.id })}>{row.task.title || "untitled"}</a> : "Nothing right now"}</dd></div>
      </dl>
      <h3>Tokens</h3>
      {secret ? <SecretOnce title={secret.title} secret={secret.value} onDone={() => setSecret(null)} /> : null}
      {tokens.status === "loading" ? <p className="muted">Loading tokens…</p> : null}
      {tokens.status === "error" ? <p className="err" role="alert">{tokens.error}</p> : null}
      {tokens.status === "ready" && !tokens.value?.length ? <p className="muted">No tokens for this member.</p> : null}
      <ul className="tokens">
        {(tokens.value ?? []).map((t) => {
          const state = tokenState(t, now);
          const asking = confirm?.id === t.id ? confirm.op : null;
          return (
            <li key={t.id} className={`token ${state}`}>
              <div className="token-head">
                <b>{t.label || "Unlabelled token"}</b>
                {t.id === pageToken ? <span className="state-chip">This page</span> : null}
                <span className={`state-chip ${state}`}>{state === "active" ? "Active" : state === "revoked" ? "Revoked" : "Expired"}</span>
              </div>
              <p className="muted small">Made {ago(t.createdAt, now)}{t.expiresAt ? ` · ${state === "expired" ? "expired" : "expires"} ${new Date(t.expiresAt).toLocaleDateString()}` : " · no expiry"}</p>
              <ul className="caps" aria-label="Capabilities">
                {t.capabilities.map((c) => <li key={c}><code>{c}</code></li>)}
              </ul>
              {state === "active" && canWrite ? (
                <div className="token-actions">
                  <button type="button" className={`chip-btn${asking === "rotate" ? " confirm" : ""}`} disabled={busy} onClick={() => tokenAct(t.id, "rotate")}>{asking === "rotate" ? "Rotate: sure?" : "Rotate"}</button>
                  <button type="button" className={`chip-btn danger${asking === "revoke" ? " confirm" : ""}`} disabled={busy} onClick={() => tokenAct(t.id, "revoke")}>{asking === "revoke" ? "Revoke: sure?" : "Revoke"}</button>
                  {asking && t.id === pageToken ? <span className="muted small">This page runs on this token. {asking === "revoke" ? "Revoking it signs you out here." : "Rotating it may sign you out here."}</span> : null}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
      {canWrite && workspace ? (
        <MintForm busy={busy} held={held.value ?? null} onMint={(req) => void run(async () => {
          const out = await api.mint(workspace, p.id, req);
          setSecret({ title: `New token for ${p.name}`, value: out.secret });
        }, "Token minted", "Could not mint the token")} />
      ) : (
        <p className="muted small">Minting, rotating and revoking need a bearer token with token:admin. A signed-in session cannot.</p>
      )}
    </div>
  );
}

function MintForm({ busy, held, onMint }: { busy: boolean; held: string[] | null; onMint: (req: { label: string; capabilities: string[]; preset: string | null }) => void }) {
  const [label, setLabel] = useState("");
  const [caps, setCaps] = useState("");
  const [preset, setPreset] = useState(true);
  const over = preset ? [] : capsExceeding(parseCaps(caps), held);
  return (
    <form
      className="stack-form mint"
      onSubmit={(e) => {
        e.preventDefault();
        onMint({ label, capabilities: parseCaps(caps), preset: preset ? WORKER_PRESET : null });
      }}
    >
      <h3>Mint a token</h3>
      <label className="field">Label<input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="What it is for" /></label>
      <label className="check"><input type="checkbox" checked={preset} onChange={(e) => setPreset(e.target.checked)} /> Agent worker preset</label>
      {!preset ? <label className="field">Capabilities<input value={caps} onChange={(e) => setCaps(e.target.value)} placeholder="thread:read, message:write" spellCheck={false} autoCapitalize="off" /></label> : null}
      {over.length ? <p className="err small" role="alert">Your token does not hold {over.join(", ")}, so it cannot grant {over.length === 1 ? "it" : "them"}.</p> : null}
      <button className="button steel" disabled={busy || over.length > 0}>Mint</button>
      <p className="muted small">A token cannot hold more than the token you act with.</p>
    </form>
  );
}

// ---- Swarm -----------------------------------------------------------------

function Swarm({ rows, people, api, workspace, presence }: { rows: RosterRow[]; people: Person[]; api: PeopleApi; workspace: string | null; presence: Map<string, Presence> }) {
  const { session } = useSession();
  const { data, pause, resume } = useSwarm(session, api);
  const names = useMemo(() => new Map(people.map((p) => [p.id, p.name])), [people]);
  const name = (id: string) => names.get(id) ?? `member ${id.slice(0, 8)}`;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 15000);
    return () => clearInterval(t);
  }, []);
  return (
    <div className="swarm">
      <Builder rows={rows} people={people} workspace={workspace} presence={presence} />
      <div className="swarm-lists">
        <Section title="Usage this month" loaded={data.usage}>
          {(u) => (
            <>
              <ul className="swarm-list">
                {u.meters.map((m) => (
                  <li key={m.meter} className="swarm-row">
                    <span>{METER_LABELS[m.meter] ?? m.meter}</span>
                    <span className="num">{m.quantity.toLocaleString()}{m.allowance != null ? <span className="muted"> of {m.allowance.toLocaleString()}</span> : null}</span>
                    {m.over ? <span className="state-chip expired">Past the free allowance</span> : null}
                  </li>
                ))}
              </ul>
              <p className="muted small">Nothing is cut off or billed; past an allowance, the workspace's admins are told.</p>
            </>
          )}
        </Section>
        <Section title="Live claims" loaded={data.claims} empty="No live claims. Nothing is being worked on right now.">
          {(rows) => (
            <ul className="swarm-list">
              {rows.map((c) => (
                <li key={`${c.thread_id}-${c.repo}`} className="swarm-row col">
                  <span><b>{name(c.assignee_id)}</b> holds <code>{c.repo}</code> <span className="muted">({c.mode}, {c.lease_expires_at ? leaseLeft(c.lease_expires_at, now) : "no lease"})</span></span>
                  {c.items?.length ? <ul className="items">{c.items.map((i) => <li key={`${i.kind}:${i.value}`}><code>{i.kind}</code> {i.value}</li>)}</ul> : null}
                </li>
              ))}
            </ul>
          )}
        </Section>
        <Section title="Held back" loaded={data.deferred} empty="No thread is being held back by its footprint.">
          {(rows) => (
            <ul className="swarm-list">
              {rows.map((d) => (
                <li key={d.thread_id} className="swarm-row col">
                  <a href={convHash({ kind: "task", id: d.thread_id })}><b>{d.title || `thread ${d.thread_id.slice(0, 8)}`}</b></a>
                  <span className="muted">{d.reasons.map((r) => REASON_LABELS[r] ?? r).join("; ")}</span>
                </li>
              ))}
            </ul>
          )}
        </Section>
        <Section title="Build slots" loaded={data.slots} empty="No slots in use.">
          {(rows) => (
            <ul className="swarm-list">
              {rows.map((s) => {
                const full = s.capacity != null && s.in_use >= s.capacity;
                return (
                  <li key={s.slot} className="swarm-row">
                    <code>{s.slot}</code>
                                        <span className="num">{s.in_use}{s.capacity == null ? <span className="muted"> · no limit</span> : <span className="muted"> of {s.capacity}</span>}</span>
                    {full ? <span className="state-chip expired">Full</span> : null}
                  </li>
                );
              })}
            </ul>
          )}
        </Section>
        <Pauses loaded={data.pauses} name={name} now={now} pause={pause} resume={resume} />
      </div>
    </div>
  );
}

function Section<T>({ title, loaded, empty, children }: { title: string; loaded: Loaded<T>; empty?: string; children: (v: T) => ReactNode }) {
  const v = loaded.value;
  const isEmpty = Array.isArray(v) && v.length === 0;
  return (
    <section className="card swarm-card" aria-label={title}>
      <h3>{title}</h3>
      {loaded.status === "loading" && v == null ? <p className="muted">Loading…</p> : null}
      {loaded.error ? <p className={v != null ? "stale" : "muted"} role={v != null ? "alert" : undefined}>{v != null ? `Last refresh failed: ${loaded.error}` : loaded.error}</p> : null}
      {v != null && isEmpty && empty ? <p className="muted">{empty}</p> : null}
      {v != null && !isEmpty ? children(v) : null}
    </section>
  );
}

function Pauses({ loaded, name, now, pause, resume }: { loaded: Loaded<Pause[]>; name: (id: string) => string; now: number; pause: (repo: string, reason: string) => Promise<void>; resume: (repo: string) => Promise<void> }) {
  const [repo, setRepo] = useState("");
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  async function run(key: string, f: () => Promise<void>) {
    setBusy(key);
    setError(null);
    try {
      await f();
      if (key === "pause") {
        setRepo("");
        setReason("");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "That did not go through");
    } finally {
      setBusy(null);
    }
  }
  return (
    <>
      <Section title="Paused repositories" loaded={loaded} empty="No repositories are paused.">
        {(rows) => (
          <ul className="swarm-list">
            {rows.map((p) => (
              <li key={p.repo} className="swarm-row">
                <span className="grow"><code>{p.repo}</code> <span className="muted">paused by {name(p.paused_by)}, {ago(p.paused_at, now)}</span>{p.reason ? <span className="muted"> · {p.reason}</span> : null}</span>
                <button type="button" className="chip-btn" disabled={busy !== null} onClick={() => void run(p.repo, () => resume(p.repo))}>{busy === p.repo ? "Resuming…" : "Resume"}</button>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <form
        className="card swarm-card pause-form"
        aria-label="Pause a repository"
        onSubmit={(e) => {
          e.preventDefault();
          void run("pause", () => pause(repo, reason));
        }}
      >
        <h3>Pause a repository</h3>
        <p className="muted small">Workers stop claiming its work until it is resumed. Claims already held run on.</p>
        <label>Repository<input value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="owner/name" autoCapitalize="off" autoCorrect="off" spellCheck={false} /></label>
        <label>Reason (optional)<input value={reason} onChange={(e) => setReason(e.target.value)} /></label>
        {error ? <p className="err" role="alert">{error}</p> : null}
        <button type="submit" className="button steel" disabled={busy !== null}>{busy === "pause" ? "Pausing…" : "Pause"}</button>
      </form>
    </>
  );
}

// ---- Builder ---------------------------------------------------------------

const ENGRAVE = "MDN-01";
type Drag = { id: string; x: number; y: number; over: Role | null };

function wellAt(x: number, y: number): Role | null {
  const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-well]");
  const r = el?.dataset.well;
  return r && (ROLES as readonly string[]).includes(r) ? (r as Role) : null;
}

function Builder({ rows, people, workspace, presence }: { rows: RosterRow[]; people: Person[]; workspace: string | null; presence: Map<string, Presence> }) {
  const [seats, setSeats] = useState<Seats>(NO_SEATS);
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const key = workspace && people.length ? `${workspace}|${people.length}` : null;
  if (key && key !== loadedFor) {
    // Adjust during render: read the saved plan once the members are known.
    setLoadedFor(key);
    setSeats(loadSeats(window.localStorage, workspace as string, people));
  }
  const [picked, setPicked] = useState<string | null>(null);
  const [focusWell, setFocusWell] = useState<Role>("planner");
  const [say, setSay] = useState<string | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const wellRefs = useRef<Partial<Record<Role, HTMLButtonElement | null>>>({});
  const byId = useMemo(() => new Map(people.map((p) => [p.id, p])), [people]);
  const c = circuit(seats);
  const presenceOf = (r: Role) => {
    const id = seats[r];
    return id ? (presence.get(id) ?? "offline") : "offline";
  };
  const status = say ?? circuitStatus(c, presenceOf);

  function commit(next: Seats) {
    setSeats(next);
    if (workspace) saveSeats(window.localStorage, workspace, next);
  }
  function place(role: Role, id: string) {
    const refusal = seatRefusal(seats, role, byId.get(id));
    if (refusal) {
      setSay(refusal);
      return;
    }
    setSay(null);
    commit(seat(seats, role, id));
    setPicked(null);
  }
  function clear(role: Role) {
    if (!seats[role]) return;
    setSay(`${ROLE_NAME[role]} seat cleared.`);
    commit(unseat(seats, role));
  }

  function onPuckDown(e: ReactPointerEvent<HTMLButtonElement>, id: string) {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ id, x: e.clientX, y: e.clientY, over: null });
  }
  function onPuckMove(e: ReactPointerEvent<HTMLButtonElement>) {
    if (!drag) return;
    setDrag({ ...drag, x: e.clientX, y: e.clientY, over: wellAt(e.clientX, e.clientY) });
  }
  function onPuckUp(e: ReactPointerEvent<HTMLButtonElement>) {
    if (!drag) return;
    const moved = Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 0 || drag.over !== null;
    const target = wellAt(e.clientX, e.clientY);
    const id = drag.id;
    setDrag(null);
    if (target) place(target, id);
    else if (!moved) setPicked((p) => (p === id ? null : id));
  }

  function onWellKey(e: KeyboardEvent<HTMLButtonElement>, role: Role) {
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      clear(role);
      return;
    }
    if (e.key === "Escape") {
      setPicked(null);
      setSay("Pick cancelled.");
      return;
    }
    const next = nextWell(role, e.key);
    if (next !== role) {
      e.preventDefault();
      setFocusWell(next);
      wellRefs.current[next]?.focus();
    }
  }
  function onWellClick(role: Role) {
    setFocusWell(role);
    if (picked) place(role, picked);
    else if (seats[role]) clear(role);
    else setSay(`Pick a member from the tray first, then choose the ${ROLE_NAME[role]} well.`);
  }

  const seated = new Set(ROLES.map((r) => seats[r]).filter(Boolean));
  const tray = rows.filter((r) => !seated.has(r.person.id));
  const dragging = drag ? byId.get(drag.id) : undefined;

  return (
    <section className="builder card" aria-label="Swarm builder">
      <div className="builder-head">
        <h3>Swarm builder</h3>
        <p className="muted small">A plan kept in this browser. Seating someone here grants them nothing.</p>
      </div>
      <div className="tile-wrap">
        <div className={`tile${c.complete ? " closed" : ""}`}>
          <span className="screw s1" aria-hidden="true" />
          <span className="screw s2" aria-hidden="true" />
          <span className="screw s3" aria-hidden="true" />
          <span className="screw s4" aria-hidden="true" />
          <span className={`lamp ${c.complete ? "ok" : c.jammed ? "bad" : ""}`} aria-hidden="true" />
          <div className="wells" role="group" aria-label="Roles. Arrow keys move between wells, Enter places the picked member, Delete clears.">
            {ROLES.map((r) => {
              const id = seats[r];
              const p = id ? byId.get(id) : undefined;
              return (
                <button
                  key={r}
                  type="button"
                  ref={(el) => {
                    wellRefs.current[r] = el;
                  }}
                  data-well={r}
                  className={`well${drag?.over === r ? " over" : ""}${c.broken === r ? " gap" : ""}`}
                  tabIndex={focusWell === r ? 0 : -1}
                  aria-label={`${ROLE_NAME[r]} well, ${p ? `${p.name} seated` : "empty"}`}
                  onKeyDown={(e) => onWellKey(e, r)}
                  onClick={() => onWellClick(r)}
                >
                  {p ? (
                    <span className={`puck ${r}`}>
                      <span className="puck-label">{initials(p.name)}</span>
                    </span>
                  ) : (
                    <span className="well-label">{ROLE_NAME[r]}</span>
                  )}
                </button>
              );
            })}
          </div>
          <span className="tile-engrave" aria-hidden="true">{ENGRAVE} · {c.placed}/4</span>
        </div>
      </div>
      <p className="builder-status" role="status" aria-live="polite">{status}</p>
      <div className="tray" role="group" aria-label="Members to seat">
        {tray.length ? null : <p className="muted small">Everyone is seated.</p>}
        {tray.map((r) => (
          <button
            key={r.person.id}
            type="button"
            className={`tray-puck ${r.person.kind === "human" ? "human" : "agent"}${picked === r.person.id ? " picked" : ""}${drag?.id === r.person.id ? " lifting" : ""}`}
            aria-pressed={picked === r.person.id}
            aria-label={`${r.person.name}, ${r.person.kind === "human" ? "human" : "agent"}, ${PRESENCE_WORD[r.presence]}`}
            onPointerDown={(e) => onPuckDown(e, r.person.id)}
            onPointerMove={onPuckMove}
            onPointerUp={onPuckUp}
            onPointerCancel={() => setDrag(null)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                const next = picked === r.person.id ? null : r.person.id;
                setPicked(next);
                setSay(next ? `${r.person.name} picked. Choose a well with the arrow keys and press Enter.` : null);
                if (next) wellRefs.current[focusWell]?.focus();
              }
            }}
          >
            <Avatar person={r.person} presence={r.presence} />
            <span>{r.person.name}</span>
          </button>
        ))}
      </div>
      {drag && dragging ? <DragGhost drag={drag} name={dragging.name} /> : null}
    </section>
  );
}

/** The puck under a finger. Positioned through a ref so no inline style attribute is set from markup. */
function DragGhost({ drag, name }: { drag: Drag; name: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el) el.style.transform = `translate(${drag.x - 24}px, ${drag.y - 24}px)`;
  }, [drag.x, drag.y]);
  return (
    <span ref={ref} className="drag-ghost puck" aria-hidden="true">
      <span className="puck-label">{initials(name)}</span>
    </span>
  );
}
