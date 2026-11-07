// One channel is the usual start of a workspace. A menu of a single option
// looks like a choice that is not there, so that case is a label. A
// remembered id that is no longer in the list clears the select until the
// reader picks, instead of showing the first channel as if it were chosen.
// No channels keeps the disabled menu: the list is empty while it loads too,
// and "no channels" would be a claim the board cannot make yet.
import type { Channel } from "../core/board.ts";
import { channelControl } from "../core/boardQuery.ts";
import { Icon } from "./icons.tsx";

export function ChannelPick({ channels, channelId, onChange }: { channels: Channel[]; channelId: string | null; onChange: (id: string) => void }) {
  if (channelControl(channels.length) === "calm") {
    const name = channels[0]?.name?.trim() || "channel";
    return (
      <span className="channel-pick channel-only" aria-label={`Channel ${name}`} title={name}>
        <span className="channel-hash" aria-hidden="true">#</span>
        <span className="channel-name">{name}</span>
      </span>
    );
  }
  const known = channelId != null && channels.some((c) => c.id === channelId);
  const orphan = Boolean(channelId) && !known && channels.length > 0;
  const value = known ? (channelId as string) : "";
  return (
    <label className="channel-pick">
      <span className="visually-hidden">Channel</span>
      <span className="channel-hash" aria-hidden="true">#</span>
      <select
        value={value}
        onChange={(e) => {
          const next = e.target.value;
          if (next) onChange(next);
        }}
        disabled={!channels.length}
        aria-invalid={orphan || undefined}
      >
        {channels.length > 0 && !value ? (
          <option value="" disabled>
            Select a channel
          </option>
        ) : null}
        {channels.map((c) => (
          <option key={c.id} value={c.id}>{c.name}</option>
        ))}
      </select>
      <Icon name="chevron" />
    </label>
  );
}
