// One channel is the usual start of a workspace. A menu of a single option
// looks like a choice that is not there, so that case is a label.
import type { Channel } from "../core/board.ts";
import { channelControl } from "../core/boardQuery.ts";
import { Icon } from "./icons.tsx";

export function ChannelPick({ channels, channelId, onChange }: { channels: Channel[]; channelId: string | null; onChange: (id: string) => void }) {
  if (channelControl(channels.length) === "calm") {
    const name = channels[0]?.name ?? "";
    return (
      <span className="channel-pick channel-only" aria-label={`Channel ${name}`}>
        <span className="channel-hash" aria-hidden="true">#</span>
        <span className="channel-name">{name}</span>
      </span>
    );
  }
  return (
    <label className="channel-pick">
      <span className="visually-hidden">Channel</span>
      <span className="channel-hash" aria-hidden="true">#</span>
      <select value={channelId ?? ""} onChange={(e) => onChange(e.target.value)} disabled={!channels.length}>
        {channels.map((c) => (
          <option key={c.id} value={c.id}>{c.name}</option>
        ))}
      </select>
      <Icon name="chevron" />
    </label>
  );
}
