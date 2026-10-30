import { useCallback, useEffect, useRef, useState } from "react";
import { liveBus } from "../core/bus.ts";
import { ThreadsApi, ThreadsError, movesList, touchesThread, type ConvRef, type Dm, type GroupDm, type Message, type Reaction } from "../core/threads.ts";
import type { SessionState } from "../core/session.ts";
import { backoff } from "./useNeeds.ts";

export type DirectData = { status: "idle" | "loading" | "ready" | "error"; dms: Dm[]; groups: GroupDm[]; error: string | null };

/** The signed-in member's DMs and group DMs, kept current by the live bus. */
export function useDirect(session: SessionState, api: ThreadsApi) {
  const me = session.status === "signed-in" ? session.memberId : null;
  const workspace = session.status === "signed-in" ? session.workspaceId : null;
  const [data, setData] = useState<DirectData>({ status: "idle", dms: [], groups: [], error: null });
  const gen = useRef(0);

  const load = useCallback(async () => {
    if (!me || !workspace) return;
    const mine = ++gen.current;
    try {
      const [dms, groups] = await Promise.all([api.dms(workspace, me), api.groupDms(workspace, me)]);
      if (mine !== gen.current) return;
      setData({ status: "ready", dms, groups, error: null });
    } catch (e) {
      if (mine !== gen.current) return;
      const message = e instanceof ThreadsError ? e.message : "Could not reach the server";
      setData((d) => ({ ...d, status: d.status === "ready" ? "ready" : "error", error: message }));
    }
  }, [me, workspace, api]);

  useEffect(() => {
    const t = setTimeout(() => void load(), 0);
    return () => clearTimeout(t);
  }, [load]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      if (m.type !== "resync" && !movesList(m.frame.kind)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void load(), 250);
    });
    return () => {
      off();
      if (timer) clearTimeout(timer);
    };
  }, [load]);

  return { data, refresh: load };
}

export type ConversationData = {
  status: "idle" | "loading" | "ready" | "error";
  messages: Message[];
  reactions: Record<string, Reaction[]>;
  /** The last read failed: what is shown is from this time. */
  staleSince: number | null;
  error: string | null;
};

const EMPTY: ConversationData = { status: "idle", messages: [], reactions: {}, staleSince: null, error: null };

/** One conversation's messages and reactions, refreshed when its thread moves. */
export function useConversation(api: ThreadsApi, threadId: string | null, me: string | null) {
  const [data, setData] = useState<ConversationData>(EMPTY);
  const gen = useRef(0);
  const failures = useRef(0);
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const again = useRef<() => Promise<void>>(async () => {});

  const load = useCallback(async () => {
    if (!threadId) return;
    const mine = ++gen.current;
    if (retry.current) clearTimeout(retry.current);
    try {
      const messages = await api.messages(threadId);
      const pairs = await Promise.all(
        messages.map(async (m) => {
          try {
            return [m.id, await api.reactions(m.id, me)] as const;
          } catch {
            return [m.id, [] as Reaction[]] as const;
          }
        }),
      );
      if (mine !== gen.current) return;
      failures.current = 0;
      setData({ status: "ready", messages, reactions: Object.fromEntries(pairs), staleSince: null, error: null });
    } catch (e) {
      if (mine !== gen.current) return;
      failures.current += 1;
      const message = e instanceof ThreadsError ? e.message : "Could not reach the server";
      setData((d) => (d.status === "ready" ? { ...d, staleSince: d.staleSince ?? Date.now(), error: message } : { ...EMPTY, status: "error", error: message }));
      retry.current = setTimeout(() => void again.current(), backoff(failures.current));
    }
  }, [api, threadId, me]);

  useEffect(() => {
    again.current = load;
  }, [load]);

  useEffect(() => {
    const t = setTimeout(() => {
      setData({ ...EMPTY, status: threadId ? "loading" : "idle" });
      void load();
    }, 0);
    return () => {
      clearTimeout(t);
      if (retry.current) clearTimeout(retry.current);
    };
  }, [load, threadId]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      if (m.type !== "resync" && !touchesThread(m.frame, threadId)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void load(), 300);
    });
    return () => {
      off();
      if (timer) clearTimeout(timer);
    };
  }, [load, threadId]);

  /** Posts, then reloads. Throws a sentence the composer shows; the draft stays. */
  const send = useCallback(
    async (ref: ConvRef, body: string) => {
      await api.post(ref, body);
      await load();
    },
    [api, load],
  );

  const react = useCallback(
    async (messageId: string, emoji: string, on: boolean) => {
      // Shown at once; the reload puts the server's count back.
      setData((d) => {
        const cur = d.reactions[messageId] ?? [];
        const found = cur.find((r) => r.emoji === emoji);
        const next = found
          ? cur.map((r) => (r.emoji === emoji ? { ...r, count: r.count + (on ? 1 : -1), mine: on } : r)).filter((r) => r.count > 0)
          : on
            ? [...cur, { emoji, count: 1, mine: true }]
            : cur;
        return { ...d, reactions: { ...d.reactions, [messageId]: next } };
      });
      try {
        await api.react(messageId, emoji, on);
      } finally {
        await load();
      }
    },
    [api, load],
  );

  return { data, send, react, refresh: load };
}

/** Sets --vv-top, --vv-h and --kb from the visual viewport, so a composer sits above the keyboard. */
export function useVisualViewport(inset: (innerHeight: number, vv: VisualViewport | null) => number) {
  useEffect(() => {
    const vv = window.visualViewport;
    const root = document.documentElement;
    const apply = () => {
      root.style.setProperty("--vv-top", `${vv ? vv.offsetTop : 0}px`);
      root.style.setProperty("--vv-h", `${vv ? vv.height : window.innerHeight}px`);
      root.style.setProperty("--kb", `${inset(window.innerHeight, vv ?? null)}px`);
    };
    apply();
    vv?.addEventListener("resize", apply);
    vv?.addEventListener("scroll", apply);
    window.addEventListener("resize", apply);
    return () => {
      vv?.removeEventListener("resize", apply);
      vv?.removeEventListener("scroll", apply);
      window.removeEventListener("resize", apply);
      for (const k of ["--vv-top", "--vv-h", "--kb"]) root.style.removeProperty(k);
    };
  }, [inset]);
}
