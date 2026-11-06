// One channel is the usual start of a workspace. A menu of a single option
// looks like a choice that is not there, so that case is a label. Zero
// channels is a calm empty label, not a disabled blank select. A remembered
// id that is no longer in the list clears the select until the reader picks.
import type { Channel } from "../core/board.ts";
import { channelPickView } from "./emptyErrorChannel.ts";
import { Icon } from "./icons.tsx";

export { channelPickView } from "./emptyErrorChannel.ts";
export type { ChannelPickView } from "./emptyErrorChannel.ts";

export function ChannelPick({ channels, channelId, onChange }: { channels: Channel[]; channelId: string | null; onChange: (id: string) => void }) {
  const view = channelPickView(channels, channelId);
  if (view.kind === "only") {
    return (
      <span className="channel-pick channel-only" aria-label={`Channel ${view.name}`} title={view.name}>
        <span className="channel-hash" aria-hidden="true">#</span>
        <span className="channel-name">{view.name}</span>
      </span>
    );
  }
  if (view.kind === "empty") {
    return (
      <span className="channel-pick channel-only" role="status" aria-label="No channels yet">
        <span className="channel-hash" aria-hidden="true">#</span>
        <span className="channel-name muted">No channels yet</span>
      </span>
    );
  }
  return (
    <label className="channel-pick">
      <span className="visually-hidden">Channel</span>
      <span className="channel-hash" aria-hidden="true">#</span>
      <select
        value={view.value}
        onChange={(e) => {
          const next = e.target.value;
          if (next) onChange(next);
        }}
        aria-invalid={view.orphan || undefined}
      >
        {view.orphan || !view.value ? (
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
