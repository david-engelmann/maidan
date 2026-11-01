import { useCallback, useEffect, useRef, useState } from "react";
import { liveBus } from "../core/bus.ts";
import { NeedsApi, NeedsError, movesNeeds, type GateView, type Need } from "../core/needs.ts";
import type { SessionState } from "../core/session.ts";

export type NeedsData = {
  status: "idle" | "loading" | "ready" | "error";
  items: Need[];
  gates: Map<string, GateView>;
  /** The last read failed: the rows shown are from this time. */
  staleSince: number | null;
  error: string | null;
};

const EMPTY: NeedsData = { status: "idle", items: [], gates: new Map(), staleSince: null, error: null };

/** The next retry after a failed read: 5 s, doubling, capped at a minute. */
export function backoff(failures: number): number {
  return Math.min(60000, 5000 * 2 ** Math.max(0, failures - 1));
}

/** What waits on the signed-in member, kept current by the live bus. */
export function useNeeds(session: SessionState, api: NeedsApi) {
  const [data, setData] = useState<NeedsData>(EMPTY);
  const me = session.status === "signed-in" ? session.memberId : null;
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const gen = useRef(0);
  const failures = useRef(0);
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  // The retry calls the current load, whichever render made it.
  const again = useRef<(quiet: boolean) => Promise<void>>(async () => {});

  const load = useCallback(
    async (quiet: boolean) => {
      if (!me) return;
      const mine = ++gen.current;
      if (retry.current) clearTimeout(retry.current);
      if (!quiet) setData((d) => ({ ...d, status: d.status === "ready" ? "ready" : "loading" }));
      try {
        const [items, gates] = await Promise.all([api.waiting(me), workspace ? api.gates(workspace) : Promise.resolve(new Map<string, GateView>())]);
        if (mine !== gen.current) return;
        failures.current = 0;
        setData({ status: "ready", items, gates, staleSince: null, error: null });
      } catch (e) {
        if (mine !== gen.current) return;
        failures.current += 1;
        const refused = e instanceof NeedsError;
        const message = e instanceof Error ? e.message : "Could not load what needs you";
        setData((d) =>
          d.status === "ready"
            ? { ...d, staleSince: d.staleSince ?? Date.now(), error: refused ? message : null }
            : { ...d, status: "error", error: refused ? message : "Could not reach the server" },
        );
        retry.current = setTimeout(() => void again.current(true), backoff(failures.current));
      }
    },
    [me, workspace, api],
  );

  useEffect(() => {
    again.current = load;
  }, [load]);

  useEffect(() => {
    const t = setTimeout(() => void load(false), 0);
    return () => {
      clearTimeout(t);
      if (retry.current) clearTimeout(retry.current);
    };
  }, [load]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      if (m.type !== "resync" && !movesNeeds(m.frame.kind)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void load(true), 250);
    });
    return () => {
      off();
      if (timer) clearTimeout(timer);
    };
  }, [load]);

  const refresh = useCallback(() => load(true), [load]);
  return { data, refresh };
}
