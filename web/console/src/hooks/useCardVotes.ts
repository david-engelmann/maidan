import { useEffect, useRef, useState, useMemo } from "react";
import type { BoardApi, VoteBadge } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";

/** How many card-vote reads run at once. A long lane must not open one request per card. */
const VOTE_READ_LIMIT = 6;

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
    let running = 0;
    const waiters: Array<() => void> = [];
    const acquire = () =>
      new Promise<void>((resolve) => {
        if (running < VOTE_READ_LIMIT) {
          running += 1;
          resolve();
        } else waiters.push(() => {
          running += 1;
          resolve();
        });
      });
    const release = () => {
      running -= 1;
      const next = waiters.shift();
      if (next) next();
    };
    const read = async (id: string) => {
      const mine = (seq.get(id) ?? 0) + 1;
      seq.set(id, mine);
      await acquire();
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
        // A blip keeps the last badge. Dropping it would flash a card as unvoted.
        if (!live || seq.get(id) !== mine) return;
      } finally {
        release();
      }
    };
    const wanted = key ? key.split(",") : [];
    for (const id of wanted) void read(id);
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
  // A card that leaves the read set loses its badge, since it would never refresh.
  return useMemo(() => {
    const wanted = new Set(key ? key.split(",") : []);
    if ([...votes.keys()].every((id) => wanted.has(id))) return votes;
    return new Map([...votes].filter(([id]) => wanted.has(id)));
  }, [votes, key]);
}
