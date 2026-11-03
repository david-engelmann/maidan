import { useEffect, useRef, useState } from "react";
import type { BoardApi, VoteBadge } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";

/** Vote counts for the given cards, refreshed when a vote on one moves. */
export function useCardVotes(api: BoardApi, threadIds: string[]): Map<string, VoteBadge> {
  const [votes, setVotes] = useState<Map<string, VoteBadge>>(() => new Map());
  const key = threadIds.join(",");
  const ids = useRef(threadIds);
  useEffect(() => {
    ids.current = threadIds;
  });

  useEffect(() => {
    let live = true;
    // Each card's latest read wins; an older response that lands late is dropped.
    const seq = new Map<string, number>();
    const read = async (id: string) => {
      const mine = (seq.get(id) ?? 0) + 1;
      seq.set(id, mine);
      try {
        const b = await api.cardVotes(id);
        if (!live || seq.get(id) !== mine) return;
        setVotes((m) => {
          const next = new Map(m);
          if (b) next.set(id, b);
          else next.delete(id);
          return next;
        });
      } catch {
        // A failed read drops the badge rather than keep a count that may be stale.
        if (!live || seq.get(id) !== mine) return;
        setVotes((m) => {
          if (!m.has(id)) return m;
          const next = new Map(m);
          next.delete(id);
          return next;
        });
      }
    };
    for (const id of key ? key.split(",") : []) void read(id);
    const off = liveBus.listen((m) => {
      if (m.type === "resync") {
        for (const id of ids.current) void read(id);
        return;
      }
      const k = m.frame.kind;
      const tid = m.frame.thread_id;
      if ((k === "vote_cast" || k === "vote_retracted") && typeof tid === "string" && ids.current.includes(tid)) void read(tid);
    });
    return () => {
      live = false;
      off();
    };
  }, [api, key]);
  return votes;
}
