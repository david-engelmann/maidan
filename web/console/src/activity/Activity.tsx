// Activity: a live stream of the workspace's events, newest first.
import { useEffect, useState } from "react";
import { ago } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import { ACTIVITY_GROUPS, describeKind, filterActivity, mergeActivity, toActivity, type ActivityGroup, type ActivityRow } from "../core/activity.ts";
import { useToolsApi } from "../hooks/useTools.ts";
import { useSession } from "../hooks/useSession.ts";

const NONE: ActivityRow[] = [];

export function Activity() {
  const { session } = useSession();
  const api = useToolsApi();
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  const [rows, setRows] = useState<ActivityRow[]>(NONE);
  const [error, setError] = useState<string | null>(null);
  const [group, setGroup] = useState<ActivityGroup>("all");
  const [paused, setPaused] = useState(false);
  const [held, setHeld] = useState<ActivityRow[]>(NONE);
  const [now, setNow] = useState(() => Date.now());

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
    void read();
    const off = liveBus.listen((m) => {
      if (m.type === "resync") return void read();
      const row = toActivity(m.frame);
      if (!row) return;
      if (paused) setHeld((h) => mergeActivity(h, [row]));
      else setRows((cur) => mergeActivity(cur, [row]));
    });
    return () => {
      live = false;
      off();
    };
  }, [api, ws, paused]);

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
          <button onClick={resume}>Resume{held.length ? ` (${held.length} new)` : ""}</button>
        ) : (
          <button onClick={() => setPaused(true)}>Pause</button>
        )}
      </header>
      {error && <p className="muted" role="alert">{error}</p>}
      {shown.length === 0 ? (
        <p className="muted pad">{ws ? "No events yet." : "Sign in to see activity."}</p>
      ) : (
        <ol className="activity-list" aria-live={paused ? "off" : "polite"}>
          {shown.map((r) => (
            <li key={r.id} className="activity-row">
              <span className="mono-id">#{r.id}</span>
              <span className="grow">{describeKind(r.kind)}</span>
              {r.threadId ? <a href={`#/threads/t/${encodeURIComponent(r.threadId)}`}>thread {r.threadId.slice(0, 8)}</a> : r.channelId ? <span className="muted small">channel {r.channelId.slice(0, 8)}</span> : null}
              {r.actorId && <span className="muted small">by {r.actorId.slice(0, 8)}</span>}
              {r.at && <time className="muted small" dateTime={r.at}>{ago(r.at, now)}</time>}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
