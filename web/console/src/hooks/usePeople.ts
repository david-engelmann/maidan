import { useCallback, useEffect, useRef, useState } from "react";
import { controlBus, liveBus } from "../core/bus.ts";
import { PeopleApi, applyPresence, type Claim, type Deferred, type Pause, type Person, type Presence, type Slot, type Token, type Usage } from "../core/people.ts";
import type { SessionState } from "../core/session.ts";

export type Loaded<T> = { status: "loading" | "ready" | "error"; value: T | null; error: string | null };
const LOADING = { status: "loading", value: null, error: null } as const;

function message(e: unknown, fallback: string): string {
  return e instanceof Error ? e.message : fallback;
}

/** Who is live: the last snapshot, then each presence frame. */
export function usePresence(): Map<string, Presence> {
  const [map, setMap] = useState<Map<string, Presence>>(() => (controlBus.last ? (applyPresence(new Map(), controlBus.last) ?? new Map()) : new Map()));
  useEffect(() => controlBus.listen((frame) => setMap((prev) => applyPresence(prev, frame) ?? prev)), []);
  return map;
}

/** The workspace's members; a resync reloads them. */
export function usePeople(session: SessionState, api: PeopleApi) {
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const [data, setData] = useState<Loaded<Person[]>>(LOADING);
  const gen = useRef(0);
  const load = useCallback(async () => {
    if (!workspace) return;
    const mine = ++gen.current;
    try {
      const people = await api.people(workspace);
      if (mine === gen.current) setData({ status: "ready", value: people, error: null });
    } catch (e) {
      if (mine !== gen.current) return;
      // A failed refresh keeps the roster on screen and says so.
      setData((d) => (d.value ? { ...d, error: `Last refresh failed: ${message(e, "unknown")}` } : { status: "error", value: null, error: message(e, "Could not load the members") }));
    }
  }, [workspace, api]);
  useEffect(() => {
    const t = setTimeout(() => void load(), 0);
    const off = liveBus.listen((m) => {
      if (m.type === "resync" || m.frame.kind === "member_created" || m.frame.kind === "member_updated") void load();
    });
    return () => {
      clearTimeout(t);
      off();
    };
  }, [load]);
  return { data, reload: load };
}

/** One member's tokens, read when their details open. */
export function useTokens(session: SessionState, api: PeopleApi, member: string | null): Loaded<Token[]> {
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const [data, setData] = useState<{ key: string; value: Loaded<Token[]> } | null>(null);
  const key = `${workspace}|${member}`;
  useEffect(() => {
    if (!workspace || !member) return;
    let live = true;
    api.tokens(workspace, member).then(
      (rows) => live && setData({ key, value: { status: "ready", value: rows, error: null } }),
      (e: unknown) => live && setData({ key, value: { status: "error", value: null, error: message(e, "Could not load the tokens") } }),
    );
    return () => {
      live = false;
    };
  }, [api, workspace, member, key]);
  return data && data.key === key ? data.value : LOADING;
}

export type SwarmData = { usage: Loaded<Usage>; claims: Loaded<Claim[]>; deferred: Loaded<Deferred[]>; slots: Loaded<Slot[]>; pauses: Loaded<Pause[]> };
const SWARM_EMPTY: SwarmData = { usage: LOADING, claims: LOADING, deferred: LOADING, slots: LOADING, pauses: LOADING };
export const SWARM_REFRESH_MS = 30000;

/** The footprint lists. Each lands on its own, so one failure blanks none of the rest. */
export function useSwarm(session: SessionState, api: PeopleApi) {
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const [data, setData] = useState<SwarmData>(SWARM_EMPTY);
  const gen = useRef(0);
  const load = useCallback(async () => {
    if (!workspace) return;
    const mine = ++gen.current;
    const one = async <K extends keyof SwarmData>(k: K, read: () => Promise<NonNullable<SwarmData[K]["value"]>>) => {
      try {
        const value = await read();
        if (mine === gen.current) setData((d) => ({ ...d, [k]: { status: "ready", value, error: null } }));
      } catch (e) {
        if (mine === gen.current) setData((d) => ({ ...d, [k]: { status: "error", value: d[k].value, error: message(e, "Could not load this list") } }));
      }
    };
    await Promise.all([
      one("usage", () => api.usage(workspace)),
      one("claims", () => api.claims(workspace)),
      one("deferred", () => api.deferred(workspace)),
      one("slots", () => api.slots(workspace)),
      one("pauses", () => api.pauses(workspace)),
    ]);
  }, [workspace, api]);
  useEffect(() => {
    const t = setTimeout(() => void load(), 0);
    const every = setInterval(() => void load(), SWARM_REFRESH_MS);
    let soon: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      const k = m.type === "event" ? m.frame.kind ?? "" : "resync";
      if (k !== "resync" && !/claim|repo_pause|slot|thread_state_changed/.test(k)) return;
      if (soon) clearTimeout(soon);
      soon = setTimeout(() => void load(), 400);
    });
    return () => {
      clearTimeout(t);
      clearInterval(every);
      if (soon) clearTimeout(soon);
      off();
    };
  }, [load]);
  const pause = useCallback(
    async (repo: string, reason: string) => {
      if (!workspace) return;
      await api.pause(workspace, repo, reason);
      await load();
    },
    [api, workspace, load],
  );
  const resume = useCallback(
    async (repo: string) => {
      if (!workspace) return;
      await api.resume(workspace, repo);
      await load();
    },
    [api, workspace, load],
  );
  return { data, refresh: load, pause, resume };
}
