// The board's view in the hash, so a filtered board or an open task is a link:
// #/board?channel=<id>&task=<id>&agent=<member id>&state=<lane>&q=<text>.
// A task link names its channel, so it opens in whichever channel the
// recipient last chose.
import { LANES, type Card, type LaneKey } from "./board.ts";

export type BoardQuery = { task: string | null; channel: string | null; agent: string | null; state: LaneKey | null; q: string };

export const EMPTY_QUERY: BoardQuery = { task: null, channel: null, agent: null, state: null, q: "" };

export function readQuery(hash: string): BoardQuery {
  const i = hash.indexOf("?");
  if (i < 0) return EMPTY_QUERY;
  const p = new URLSearchParams(hash.slice(i + 1));
  const s = p.get("state");
  return {
    task: p.get("task") || null,
    channel: p.get("channel") || null,
    agent: p.get("agent") || null,
    state: LANES.some((l) => l.key === s) ? (s as LaneKey) : null,
    q: p.get("q") ?? "",
  };
}

export function writeQuery(q: BoardQuery): string {
  const p = new URLSearchParams();
  if (q.agent) p.set("agent", q.agent);
  if (q.state) p.set("state", q.state);
  if (q.q.trim()) p.set("q", q.q);
  if ((q.task || filtering(q)) && q.channel) p.set("channel", q.channel);
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

/** One channel is a label. None or many is the menu, disabled when there are none. */
export function channelControl(count: number): "calm" | "menu" {
  return count === 1 ? "calm" : "menu";
}

/** Heard by whichever board is mounted, so Threads can change channel without leaving. */
export const SELECT_CHANNEL_EVENT = "maidan:select-channel";

/**
 * The channel to read. A requested id wins (the menu, a resync). Otherwise a
 * link's `?channel=` wins over the remembered channel, so a palette jump lands
 * before the first read. With nothing named, the first channel.
 */
export function pickChannel(requested: string | null, channels: readonly { id: string }[], stored: string | null, fromLink: string | null): string | null {
  if (requested) return requested;
  if (fromLink && channels.some((c) => c.id === fromLink)) return fromLink;
  if (stored && channels.some((c) => c.id === stored)) return stored;
  return channels[0]?.id ?? null;
}

export function boardChannelHash(id: string): string {
  return `#/board?channel=${encodeURIComponent(id)}`;
}

/**
 * A palette channel command. Threads selects in place. Every other place opens
 * the board on that channel. An address bar that already says so selects again,
 * because writing the same hash does not fire a change.
 */
export function channelJump(route: string, hash: string, id: string): { href: string | null; select: boolean } {
  if (route === "threads") return { href: null, select: true };
  const href = boardChannelHash(id);
  if (hash === href) return { href: null, select: true };
  return { href, select: false };
}

export function followChannelJump(route: string, id: string): void {
  const jump = channelJump(route, window.location.hash, id);
  if (jump.select) window.dispatchEvent(new CustomEvent(SELECT_CHANNEL_EVENT, { detail: id }));
  if (jump.href) window.location.hash = jump.href;
}
