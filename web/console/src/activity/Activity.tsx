// Activity: a live stream of the workspace's events, newest first.
import { useEffect, useMemo, useRef, useState } from "react";
import { ago, BoardApi, type Channel, type Member, type Thread } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import {
  ACTIVITY_GROUPS,
  absorbPayloadNames,
  actorText,
  actorWords,
  channelText,
  describeActivity,
  EMPTY_DIRECTORY,
  filterActivity,
  mergeActivity,
  threadText,
  toActivity,
  type ActivityDirectory,
  type ActivityGroup,
  type ActivityRow,
} from "../core/activity.ts";
import { ThreadsApi } from "../core/threads.ts";
import { useToolsApi } from "../hooks/useTools.ts";
import { useSession } from "../hooks/useSession.ts";
import { EmptyState } from "../shell/EmptyState";
import { LoadError } from "../shell/LoadError.tsx";

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
  const api = useToolsApi();
  const board = useMemo(() => new BoardApi((u, i) => fetch(u, i), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const threads = useMemo(() => new ThreadsApi((u, i) => fetch(u, i), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  const [rows, setRows] = useState<ActivityRow[]>(NONE);
  const [dir, setDir] = useState<ActivityDirectory>(EMPTY_DIRECTORY);
  const [error, setError] = useState<string | null>(null);
  const [group, setGroup] = useState<ActivityGroup>("all");
  const [paused, setPaused] = useState(false);
  const [held, setHeld] = useState<ActivityRow[]>(NONE);
  const [now, setNow] = useState(() => Date.now());
  // Try again re-runs the read; a healthy socket sends no resync to do it.
  const [tries, setTries] = useState(0);
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const wsRef = useRef(ws);
  wsRef.current = ws;
  const mounted = useRef(true);
  // One lookup per unknown thread, including a miss, so a DM or a new task resolves once.
  const inflight = useRef(new Set<string>());
  const missed = useRef(new Set<string>());

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!ws) return;
    let live = true;
    const read = () =>
      api.recentEvents(ws).then(
        (raw) => {
          if (!live) return;
          setRows((cur) => mergeActivity(cur, raw.map(toActivity).filter((r): r is ActivityRow => r !== null)));
          setError(null);
        },
        () => live && setError("Could not load recent events."),
      );
    // Paused, the list holds still: no REST merge until Resume re-runs this.
    if (!paused) void read();
    const off = liveBus.listen((m) => {
      if (m.type === "resync") return void (paused ? undefined : read());
      const row = toActivity(m.frame);
      if (!row) return;
      if (paused) setHeld((h) => mergeActivity(h, [row]));
      else setRows((cur) => mergeActivity(cur, [row]));
    });
    return () => {
      live = false;
      off();
    };
  }, [api, ws, paused, tries]);

  useEffect(() => {
    if (!ws) return;
    let live = true;
    void (async () => {
      const [channels, members] = await Promise.all([
        board.channels(ws).catch(() => [] as Channel[]),
        board.members(ws).catch(() => [] as Member[]),
      ]);
      const batches = await mapLimited(channels, 4, (c) => board.threads(c.id).catch(() => [] as Thread[]));
      if (!live) return;
      const loadedThreads = new Map<string, string>();
      const loadedChannels = new Map<string, string>();
      const loadedActors = new Map<string, string>();
      for (const c of channels) if (c.name?.trim()) loadedChannels.set(c.id, c.name.trim());
      for (const t of batches.flat()) if (t.title?.trim()) loadedThreads.set(t.id, t.title.trim());
      for (const m of members) {
        const label = actorWords(m);
        if (label) loadedActors.set(m.id, label);
      }
      setDir((prev) => {
        const threadNames = new Map(loadedThreads);
        // A DM or a task created after this read keeps the title it already resolved.
        for (const [id, title] of prev.threads) if (!threadNames.has(id)) threadNames.set(id, title);
        return absorbPayloadNames({ threads: threadNames, channels: loadedChannels, actors: loadedActors }, rowsRef.current);
      });
    })();
    return () => {
      live = false;
    };
  }, [board, ws]);

  useEffect(() => {
    setDir((d) => absorbPayloadNames(d, rows));
  }, [rows]);

  useEffect(() => {
    if (!ws) return;
    const ids = [...new Set(rows.map((r) => r.threadId).filter((id): id is string => !!id))].filter(
      (id) => !dir.threads.has(id) && !rows.some((r) => r.threadId === id && !!r.title?.trim()) && !missed.current.has(id) && !inflight.current.has(id),
    );
    if (!ids.length) return;
    const workspace = ws;
    for (const id of ids) inflight.current.add(id);
    // Leaving the reads running when this effect re-runs is what keeps the memo:
    // a cleanup that abandoned them would fetch the same thread again.
    void mapLimited(ids, 4, async (id) => {
      try {
        const t = await threads.thread(id);
        const title = t.title?.trim() || "";
        // A workspace switch must not file this title under the new room.
        if (!mounted.current || wsRef.current !== workspace) return;
        if (title) setDir((d) => (d.threads.get(id) === title ? d : { ...d, threads: new Map(d.threads).set(id, title) }));
        else missed.current.add(id);
      } catch {
        if (mounted.current && wsRef.current === workspace) missed.current.add(id);
      } finally {
        inflight.current.delete(id);
      }
    });
  }, [rows, dir, ws, threads]);

  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);

  const resume = () => {
    setRows((cur) => mergeActivity(cur, held));
    setHeld(NONE);
    setPaused(false);
  };

  const shown = filterActivity(rows, group);
  return (
    <section className="activity" aria-labelledby="activity-title">
      <header className="activity-head">
        <h1 id="activity-title">Activity</h1>
        <div className="seg" role="group" aria-label="Show">
          {ACTIVITY_GROUPS.map((g) => (
            <button key={g.id} className={g.id === group ? "on" : ""} aria-pressed={g.id === group} onClick={() => setGroup(g.id)}>
              {g.title}
            </button>
          ))}
        </div>
        {paused ? (
          <button type="button" className="button ghost" onClick={resume}>Resume{held.length ? ` (${held.length} new)` : ""}</button>
        ) : (
          <button type="button" className="button ghost" onClick={() => setPaused(true)}>Pause</button>
        )}
      </header>
      {error && rows.length === 0 ? (
        <LoadError message={error} onRetry={() => setTries((n) => n + 1)} />
      ) : error ? (
        <p className="muted" role="alert">{error}</p>
      ) : null}
      {error && rows.length === 0 ? null : shown.length === 0 ? (
        <EmptyState title={ws ? "No events yet" : "Not signed in"} body={ws ? "Tasks, messages and reviews show here as they happen." : "Sign in to see activity."} />
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
                {channel ? <span className="muted small">{channel}</span> : null}
                {who ? <span className="muted small">by {who}</span> : null}
                {r.at && <time className="muted small" dateTime={r.at}>{ago(r.at, now)}</time>}
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
