import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { BoardApi, movesBoard, type Channel, type Gate, type Member, type Thread } from "../core/board.ts";
import { SELECT_CHANNEL_EVENT, pickChannel, readQuery } from "../core/boardQuery.ts";
import { settleMembers } from "../core/threads.ts";
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
  /** When the board last loaded, and when refreshes started failing. */
  loadedAt: number | null;
  failedSince: number | null;
};

const CHANNEL_KEY = "maidan_console_channel";

const EMPTY: BoardData = { status: "idle", error: null, channels: [], channelId: null, threads: [], gates: {}, members: new Map(), refusals: new Map(), fresh: new Set(), loadedAt: null, failedSince: null };

/** What the board shows the moment the room changes. Signed out is empty; a new room is loading. */
export function boardDataForRoom(workspace: string | null): BoardData {
  return {
    status: workspace ? "loading" : "idle",
    error: null,
    channels: [],
    channelId: null,
    threads: [],
    gates: {},
    members: new Map(),
    refusals: new Map(),
    fresh: new Set(),
    loadedAt: null,
    failedSince: null,
  };
}

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** The channel this workspace last had open, then the legacy single key. */
export function readStoredChannel(workspace: string, store: Pick<Storage, "getItem"> | null = storage()): string | null {
  if (!store) return null;
  try {
    return store.getItem(`${CHANNEL_KEY}:${workspace}`) ?? store.getItem(CHANNEL_KEY);
  } catch {
    return null;
  }
}

/** Remember a channel for this workspace and for a build that only reads the legacy key. */
export function writeStoredChannel(workspace: string, id: string, store: Pick<Storage, "setItem"> | null = storage()): void {
  if (!store) return;
  try {
    store.setItem(`${CHANNEL_KEY}:${workspace}`, id);
    store.setItem(CHANNEL_KEY, id);
  } catch {
    // A private window: the channel still changes for this visit.
  }
}

function sig(th: Thread): string {
  return `${th.state}|${th.assignee_id ?? ""}|${th.work_started_at ?? ""}|${th.updated_at ?? ""}`;
}

/** The board for the signed-in workspace, kept current by the live bus. */
export function useBoard(session: SessionState, api: BoardApi) {
  const [data, setData] = useState<BoardData>(EMPTY);
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const [room, setRoom] = useState(workspace);
  const seen = useRef(new Map<string, string>());
  const gen = useRef(0);
  if (room !== workspace) {
    setRoom(workspace);
    setData(boardDataForRoom(workspace));
  }
  useLayoutEffect(() => {
    gen.current += 1;
    seen.current = new Map();
  }, [workspace]);

  const load = useCallback(
    async (channelId: string | null, quiet: boolean) => {
      if (!workspace) return;
      const mine = ++gen.current;
      if (!quiet) setData((d) => ({ ...d, status: "loading", error: null }));
      try {
        const [channels, members] = await Promise.all([api.channels(workspace), api.members(workspace).catch(() => [] as Member[])]);
        const stored = readStoredChannel(workspace);
        const fromLink = typeof window !== "undefined" ? readQuery(window.location.hash).channel : null;
        const pick = pickChannel(channelId, channels, stored, fromLink);
        // A link named this channel. Remember it, or the next plain visit
        // would reopen whatever channel was stored before the link.
        if (!channelId && fromLink && pick === fromLink) writeStoredChannel(workspace, pick);
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
        setData({ status: "ready", error: null, channels, channelId: pick, threads, gates, members: settleMembers(new Map(members.map((m) => [m.id, m]))), refusals, fresh, loadedAt: Date.now(), failedSince: null });
      } catch (e) {
        if (mine !== gen.current) return;
        const message = e instanceof Error ? e.message : "Could not load the board";
        setData((d) => (quiet && d.status === "ready" ? { ...d, error: `Last refresh failed: ${message}`, failedSince: d.failedSince ?? Date.now() } : { ...d, status: "error", error: message }));
      }
    },
    [workspace, api],
  );

  // The first read for this workspace, on the next tick.
  useEffect(() => {
    if (!workspace) return;
    const t = setTimeout(() => void load(null, false), 0);
    return () => clearTimeout(t);
  }, [load, workspace]);

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
      if (workspace) writeStoredChannel(workspace, id);
      seen.current = new Map();
      void load(id, false);
    },
    [load, workspace],
  );
  // The palette asks for a channel while this screen is up. A hash change
  // covers the board; Threads has no channel in its hash, so it listens.
  useEffect(() => {
    const onPick = (e: Event) => {
      const id = e instanceof CustomEvent && typeof e.detail === "string" ? e.detail : "";
      if (id) selectChannel(id);
    };
    window.addEventListener(SELECT_CHANNEL_EVENT, onPick);
    return () => window.removeEventListener(SELECT_CHANNEL_EVENT, onPick);
  }, [selectChannel]);
  const refresh = useCallback(() => load(channelId, true), [load, channelId]);
  return { data, selectChannel, refresh };
}
