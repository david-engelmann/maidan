import { useCallback, useEffect, useRef, useState } from "react";
import { BoardApi, movesBoard, type Channel, type Gate, type Member, type Thread } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import type { SessionState } from "../core/session.ts";

export type BoardData = {
  status: "idle" | "loading" | "ready" | "error";
  error: string | null;
  channels: Channel[];
  channelId: string | null;
  threads: Thread[];
  gates: Record<string, Gate>;
  members: Map<string, Member>;
  /** The server's sentence for each in-review task whose close it refused. */
  refusals: Map<string, string>;
  /** Ids whose card changed in the last live refresh, so it can flash once. */
  fresh: Set<string>;
};

const CHANNEL_KEY = "maidan_console_channel";

const EMPTY: BoardData = { status: "idle", error: null, channels: [], channelId: null, threads: [], gates: {}, members: new Map(), refusals: new Map(), fresh: new Set() };

function sig(th: Thread): string {
  return `${th.state}|${th.assignee_id ?? ""}|${th.work_started_at ?? ""}|${th.updated_at ?? ""}`;
}

/** The board for the signed-in workspace, kept current by the live bus. */
export function useBoard(session: SessionState, api: BoardApi) {
  const [data, setData] = useState<BoardData>(EMPTY);
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const seen = useRef(new Map<string, string>());
  const gen = useRef(0);

  const load = useCallback(
    async (channelId: string | null, quiet: boolean) => {
      if (!workspace) return;
      const mine = ++gen.current;
      if (!quiet) setData((d) => ({ ...d, status: "loading", error: null }));
      try {
        const [channels, members] = await Promise.all([api.channels(workspace), api.members(workspace).catch(() => [] as Member[])]);
        const stored = typeof localStorage !== "undefined" ? localStorage.getItem(CHANNEL_KEY) : null;
        const pick = channelId ?? (stored && channels.some((c) => c.id === stored) ? stored : (channels[0]?.id ?? null));
        const [threads, gates] = pick ? await Promise.all([api.threads(pick), api.gates(workspace)]) : [[], {}];
        const refusals = pick ? await api.refusals(workspace, pick, threads) : new Map<string, string>();
        if (mine !== gen.current) return;
        const fresh = new Set<string>();
        const next = new Map<string, string>();
        for (const th of threads) {
          const s = sig(th);
          if (quiet && seen.current.size && seen.current.get(th.id) !== s) fresh.add(th.id);
          next.set(th.id, s);
        }
        seen.current = next;
        setData({ status: "ready", error: null, channels, channelId: pick, threads, gates, members: new Map(members.map((m) => [m.id, m])), refusals, fresh });
      } catch (e) {
        if (mine !== gen.current) return;
        const message = e instanceof Error ? e.message : "Could not load the board";
        setData((d) => (quiet && d.status === "ready" ? { ...d, error: `Last refresh failed: ${message}` } : { ...d, status: "error", error: message }));
      }
    },
    [workspace, api],
  );

  // The first read for this workspace, on the next tick.
  useEffect(() => {
    const t = setTimeout(() => void load(null, false), 0);
    return () => clearTimeout(t);
  }, [load]);

  // A burst of events (an agent claiming, starting, posting) is one reload.
  const channelId = data.channelId;
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      if (m.type !== "resync" && !movesBoard(m.frame.kind)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void load(channelId, true), 250);
    });
    return () => {
      off();
      if (timer) clearTimeout(timer);
    };
  }, [load, channelId]);

  const selectChannel = useCallback(
    (id: string) => {
      localStorage.setItem(CHANNEL_KEY, id);
      seen.current = new Map();
      void load(id, false);
    },
    [load],
  );
  const refresh = useCallback(() => load(channelId, true), [load, channelId]);
  return { data, selectChannel, refresh };
}
