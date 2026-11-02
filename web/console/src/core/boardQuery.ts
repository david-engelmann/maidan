// The board's view in the hash, so a filtered board or an open task is a link:
// #/board?channel=<id>&task=<id>&agent=<member id>&state=<lane>&q=<text>.
import { LANES, type Card, type LaneKey } from "./board.ts";

export type BoardQuery = { channel: string | null; task: string | null; agent: string | null; state: LaneKey | null; q: string };

export const EMPTY_QUERY: BoardQuery = { channel: null, task: null, agent: null, state: null, q: "" };

export function readQuery(hash: string): BoardQuery {
  const i = hash.indexOf("?");
  if (i < 0) return EMPTY_QUERY;
  const p = new URLSearchParams(hash.slice(i + 1));
  const s = p.get("state");
  return {
    channel: p.get("channel") || null,
    task: p.get("task") || null,
    agent: p.get("agent") || null,
    state: LANES.some((l) => l.key === s) ? (s as LaneKey) : null,
    q: p.get("q") ?? "",
  };
}

export function writeQuery(q: BoardQuery): string {
  const p = new URLSearchParams();
  if (q.channel) p.set("channel", q.channel);
  if (q.agent) p.set("agent", q.agent);
  if (q.state) p.set("state", q.state);
  if (q.q.trim()) p.set("q", q.q);
  if (q.task) p.set("task", q.task);
  const s = p.toString();
  return s ? `#/board?${s}` : "#/board";
}

/** A card shows when it matches every filter set; text matches the title or id. */
export function matches(card: Card, q: BoardQuery): boolean {
  if (q.state && card.chrome.lane !== q.state) return false;
  if (q.agent && card.thread.assignee_id !== q.agent) return false;
  const text = q.q.trim().toLowerCase();
  if (!text) return true;
  return (card.title ?? "").toLowerCase().includes(text) || card.thread.id.toLowerCase().startsWith(text);
}

export function filtering(q: BoardQuery): boolean {
  return Boolean(q.agent || q.state || q.q.trim());
}
