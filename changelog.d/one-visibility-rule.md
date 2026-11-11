### One visibility rule for every read

- **Security:** `GET /dm/{id}/messages` now needs a participant of the DM,
  as `GET /dm/{id}` and posting to it did; any member with `workspace:read`
  and the id could read the conversation.
- **Security:** an artifact is read (`GET /artifacts/{sha}` and `/meta`,
  their console twins, MCP `get_artifact`, `get_artifact_metadata` and
  `resources/read`) only when every thread in the workspace that
  references it, by a thread link, a land-gate verdict or a private or DM
  message's metadata, is one the caller can read, or the caller uploaded
  it. Anything else is not found. A sha from a private channel no longer
  opens its file for the rest of the workspace, and linking it somewhere
  public does not unlock it.
- **Security:** the inbox, the mention list, notifications, their unread
  counts and the manager digest show only what the member can read now,
  checked when they are read rather than when they were written, so a
  mention from a private channel the member is not in stays out and
  leaving a channel or DM takes its rows away. The pending-gate lists
  (REST, the console and the waiting-on-you inbox on REST and MCP) skip
  gates on threads the caller cannot read, and reading or answering one
  (REST answer, console confirmation, MCP `get_approval_gate` and
  `approval_decide`) answers as not found.
- **Security:** `ThreadBlocked` events carry their thread and channel, so
  the event log and live streams show them only to the thread's readers;
  they were stored as workspace-wide, note included. The event location
  accessors name every variant, so a new one must say where it happened.
- **Changed:** `Store::list_pending_approval_gates` takes the reader, and
  `Store::artifact_visible_to` is the one artifact read rule.
