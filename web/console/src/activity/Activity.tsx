// Activity: a live stream of the workspace's events, newest first.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ApiError } from "../core/api.ts";
import { ago, BoardApi, type Channel, type Member, type Thread } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import {
  ACTIVITY_GROUPS,
  absorbPayloadNames,
  activityChannels,
  actorText,
  actorWords,
  channelText,
  describeActivity,
  directoryAfterLoad,
  EMPTY_DIRECTORY,
  filterActivity,
  mergeActivity,
  threadText,
  toActivity,
  type ActivityDirectory,
  type ActivityGroup,
  type ActivityRow,
} from "../core/activity.ts";
import type { SessionStore } from "../core/session.ts";
import { ThreadsApi } from "../core/threads.ts";
import { useToolsApi } from "../hooks/useTools.ts";
import { useSession } from "../hooks/useSession.ts";
import { EmptyState } from "../shell/EmptyState";
import { DataState, couldNotLoad, phaseOf } from "../shell/DataState.tsx";
import "./activity.css";

const NONE: ActivityRow[] = [];

/** A handful of reads at a time, so a workspace with many channels does not open them all at once. */
async function mapLimited<T, R>(items: T[], n: number, f: (t: T) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(items.length);
  let i = 0;
  await Promise.all(
    Array.from({ length: Math.min(n, items.length) }, async () => {
      while (i < items.length) {
        const k = i++;
        out[k] = await f(items[k]);
      }
    }),
  );
  return out;
}

export function Activity() {
  const { session, store } = useSession();
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  if (!ws) {
    return (
      <div className="activity">
        <header className="activity-head">
          <h1 id="activity-title">Activity</h1>
        </header>
        <EmptyState title="Not signed in" body="Sign in to see activity." />
      </div>
    );
  }
  return <ActivityFeed key={ws} ws={ws} store={store} />;
}

function ActivityFeed({ ws, store }: { ws: string; store: SessionStore }) {
  const api = useToolsApi();
  const board = useMemo(() => new BoardApi((u, i) => fetch(u, i), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const threads = useMemo(() => new ThreadsApi((u, i) => fetch(u, i), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const [rows, setRows] = useState<ActivityRow[]>(NONE);
  const [dir, setDir] = useState<ActivityDirectory>(EMPTY_DIRECTORY);
  const [error, setError] = useState<string | null>(null);
  // Until the first read answers, an empty list means "not yet", not "nothing".
  const [settled, setSettled] = useState(false);
  const [filter, setFilter] = useState<ActivityGroup>("all");
  const [query, setQuery] = useState("");
  const [channelId, setChannelId] = useState("");
  const [paused, setPaused] = useState(false);
  const [held, setHeld] = useState<ActivityRow[]>(NONE);
  const [now, setNow] = useState(() => Date.now());
  // Try again re-runs the read; a healthy socket sends no resync to do it.
  const [tries, setTries] = useState(0);
  const rowsRef = useRef(rows);
  useEffect(() => {
    rowsRef.current = rows;
  }, [rows]);
  const gone = useRef(false);
  // One lookup per unknown thread. A miss is a real absence; a blip may try once more.
  const inflight = useRef(new Set<string>());
  const missed = useRef(new Set<string>());
  const failed = useRef(new Map<string, number>());
  const reload = useCallback(() => {
    missed.current.clear();
    failed.current.clear();
  }, []);
  useEffect(() => {
    gone.current = false;
    return () => {
      gone.current = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const pull = () =>
      api.recentEvents(ws).then(
        (raw) => {
          if (cancelled) return;
          setRows((cur) => mergeActivity(cur, raw.map(toActivity).filter((r): r is ActivityRow => r !== null)));
          setError(null);
          setSettled(true);
        },
        (e: unknown) => {
          if (cancelled) return;
          setError(e instanceof Error && e.message ? e.message : couldNotLoad("recent events"));
          setSettled(true);
        },
      );
    // Paused, the list holds still: no REST merge until Resume re-runs this.
    if (!paused) void pull();
    const off = liveBus.listen((m) => {
      if (cancelled) return;
      if (m.type === "resync") {
        reload();
        return void (paused ? undefined : pull());
      }
      const row = toActivity(m.frame);
      if (!row) return;
      if (paused) setHeld((h) => mergeActivity(h, [row]));
      else setRows((cur) => mergeActivity(cur, [row]));
    });
    return () => {
      cancelled = true;
      off();
    };
  }, [api, ws, paused, tries, reload]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [channels, members] = await Promise.all([
        board.channels(ws).catch(() => [] as Channel[]),
        board.members(ws).catch(() => [] as Member[]),
      ]);
      const batches = await mapLimited(channels, 4, (c) => board.threads(c.id).catch(() => [] as Thread[]));
      if (cancelled) return;
      const loadedThreads = new Map<string, string>();
      const loadedChannels = new Map<string, string>();
      const loadedActors = new Map<string, string>();
      for (const c of channels) if (c.name?.trim()) loadedChannels.set(c.id, c.name.trim());
      for (const t of batches.flat()) if (t.title?.trim()) loadedThreads.set(t.id, t.title.trim());
      for (const m of members) {
        const label = actorWords(m);
        if (label) loadedActors.set(m.id, label);
      }
      setDir((prev) => directoryAfterLoad(prev, { threads: loadedThreads, channels: loadedChannels, actors: loadedActors }, rowsRef.current));
    })();
    return () => {
      cancelled = true;
    };
  }, [board, ws]);

  useEffect(() => {
    setDir((d) => absorbPayloadNames(d, rows));
  }, [rows]);

  useEffect(() => {
    const ids = [...new Set(rows.map((r) => r.threadId).filter((id): id is string => !!id))].filter(
      (id) => !dir.threads.has(id) && !rows.some((r) => r.threadId === id && !!r.title?.trim()) && !missed.current.has(id) && !inflight.current.has(id),
    );
    if (!ids.length) return;
    for (const id of ids) inflight.current.add(id);
    // Leaving the reads running when this effect re-runs is what keeps the memo:
    // a cleanup that abandoned them would fetch the same thread again. Unmount
    // still drops the answer, so it cannot land in the next workspace.
    void mapLimited(ids, 4, async (id) => {
      try {
        const t = await threads.thread(id);
        const title = t.title?.trim() || "";
        if (gone.current) return;
        if (title) {
          failed.current.delete(id);
          setDir((d) => (d.threads.get(id) === title ? d : { ...d, threads: new Map(d.threads).set(id, title) }));
        } else missed.current.add(id);
      } catch (err) {
        if (gone.current) return;
        if (err instanceof ApiError && (err.status === 404 || err.status === 403)) {
          missed.current.add(id);
          return;
        }
        const n = (failed.current.get(id) ?? 0) + 1;
        failed.current.set(id, n);
        if (n >= 2) missed.current.add(id);
      } finally {
        inflight.current.delete(id);
      }
    });
  }, [rows, dir, threads]);

  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);

  const resume = () => {
    setRows((cur) => mergeActivity(cur, held));
    setHeld(NONE);
    setPaused(false);
  };
  const clearFilters = () => {
    setFilter("all");
    setChannelId("");
    setQuery("");
  };

  const channels = useMemo(() => activityChannels(rows, dir), [rows, dir]);
  const shown = useMemo(() => filterActivity(rows, filter, query, channelId, dir), [rows, filter, query, channelId, dir]);
  return (
    <div className="activity">
      <header className="activity-head">
        <h1 id="activity-title">Activity</h1>
        <div className="seg" role="group" aria-label="Show">
          {ACTIVITY_GROUPS.map((g) => (
            <button key={g.id} type="button" className={g.id === filter ? "on" : ""} aria-pressed={g.id === filter} onClick={() => setFilter(g.id)}>
              {g.title}
            </button>
          ))}
        </div>
        <label className="field">
          <span className="visually-hidden">Filter activity</span>
          <input type="search" aria-label="Filter activity" placeholder="Filter by task, person, or event…" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
        {channels.length > 0 ? (
          <label className="field">
            <span className="visually-hidden">Channel</span>
            <select aria-label="Channel" value={channelId} onChange={(e) => setChannelId(e.target.value)}>
              <option value="">All channels</option>
              {channels.map((c) => <option key={c.id} value={c.id}>{c.label}</option>)}
            </select>
          </label>
        ) : null}
        {paused ? (
          <button type="button" className="button ghost" onClick={resume}>Resume{held.length ? ` (${held.length} new)` : ""}</button>
        ) : (
          <button type="button" className="button ghost" onClick={() => setPaused(true)}>Pause</button>
        )}
      </header>
      <DataState
        phase={phaseOf({ settled, error, count: rows.length })}
        label="recent events"
        error={error}
        onRetry={() => { reload(); setTries((n) => n + 1); }}
        empty={{ icon: "activity", title: "Nothing has happened yet", body: "Tasks, messages and reviews show here as they happen." }}
        stale={error}
      >
      {shown.length === 0 ? (
        <EmptyState
          title="No matching events"
          body="Nothing in this view matches the category, channel, or search."
          action={<button type="button" className="btn" onClick={clearFilters}>Show all events</button>}
        />
      ) : (
        <ol className="activity-list" aria-live={paused ? "off" : "polite"}>
          {shown.map((r) => {
            const channel = channelText(r, dir);
            const who = actorText(r, dir);
            return (
              <li key={r.id} className="activity-row">
                <span className="mono-id">#{r.id}</span>
                <span className="grow">{describeActivity(r)}</span>
                {r.threadId ? <a href={`#/threads/t/${encodeURIComponent(r.threadId)}`}>{threadText(r, dir)}</a> : null}
                {r.channelId && channel ? <a className="activity-channel muted small" href={`#/board?channel=${encodeURIComponent(r.channelId)}`}>{channel}</a> : null}
                {who ? <span className="muted small">by {who}</span> : null}
                {r.at && <time className="muted small" dateTime={r.at}>{ago(r.at, now)}</time>}
              </li>
            );
          })}
        </ol>
      )}
      </DataState>
    </div>
  );
}
