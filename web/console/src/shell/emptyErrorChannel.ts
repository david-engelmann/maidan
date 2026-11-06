import type { Channel } from "../core/board.ts";
import { channelControl } from "../core/boardQuery.ts";

export function emptyStateTitle(title: string): string {
  const t = title.trim();
  return t || "Nothing here yet";
}

export function loadErrorMessage(message: string): string {
  const m = message.trim();
  return m || "Could not load this screen.";
}

export type ChannelPickView =
  | { kind: "only"; name: string }
  | { kind: "empty" }
  | { kind: "menu"; value: string; orphan: boolean };

/** Pure view model so empty / orphan / single-channel cases stay testable. */
export function channelPickView(channels: readonly Channel[], channelId: string | null): ChannelPickView {
  if (channels.length === 0) return { kind: "empty" };
  if (channelControl(channels.length) === "calm") {
    return { kind: "only", name: channels[0]?.name?.trim() || "channel" };
  }
  const known = channelId != null && channels.some((c) => c.id === channelId);
  return { kind: "menu", value: known ? (channelId as string) : "", orphan: Boolean(channelId) && !known };
}
