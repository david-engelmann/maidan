# Cluster 308.0 retro — Slack projector: channel links + inbound routing

> Tag **`v308.0.0`**. Phase XXIV (post-gate hardening). Slack projector, part 2. No new gate tag.

## What shipped

The inbound half of the Slack projector: a Slack channel now maps to a Maidan thread, and
messages posted in Slack appear in Maidan:

- **`maidan_slack_channel_links` table** (pg 0051 / sqlite 0050) + `SlackChannelLink` /
  `NewSlackChannelLink` models — maps `slack_channel_id` → the Maidan
  `workspace_id`/`channel_id`/`thread_id` it projects into, and the `member_id` inbound Slack
  messages post as. One Maidan thread per Slack channel. Store (both backends):
  `link_slack_channel` (upsert), `get_slack_channel_link`, `list_slack_channel_links`,
  `unlink_slack_channel`.
- **Inbound routing** (`slack.rs::route_slack_event`): on an `event_callback` with a plain user
  `message` in a linked channel, the projector posts `"{slack_user}: {text}"` into the mapped
  thread (as the link's member) via `post_message_with_event` + `publish_stored` — so it flows
  through Maidan's normal event/notification path. Best-effort (the ingress always ACKs `200`;
  Slack retries on non-200).

## Surprises / decisions

- **Explicit link target, not find-or-create.** The link names the exact
  channel/thread/member, so ingress is a single `get` + `post` — no thread-discovery logic. The
  operator (or a later management API) sets up the mapping.
- **Loop prevention baked in early.** Inbound routing **skips `bot_id` messages and any
  `subtype`** (edits/deletes/joins), and stamps the posted message's metadata with
  `{"slack": {...}}`. So the egress cluster (309) can skip re-posting Slack-sourced messages,
  and a message the projector echoes to Slack (a bot message) never re-enters Maidan. The e2e
  proves a `bot_id` message is *not* re-projected.
- **Flows through the normal path.** Projected messages use `post_message_with_event` +
  `publish_stored` (not a bypass), so they get event-log/notification/search treatment like any
  Maidan message — the projector is a thin producer, not a special case.

## Capability table extension

`maidan_slack_channel_links` + store (link/get/list/unlink); Slack `message` events in a linked
channel now post into the mapped Maidan thread. No new capability/route (links are store-level
in this cluster; a management API can follow).

## Risks identified + still open

- **Link management is store-level only** — no REST/MCP to create links yet (tests seed via the
  store). A management surface can follow if needed; the projector is functional once a link
  exists.
- **Egress is 309** — Maidan messages don't yet appear in Slack. The metadata origin-tag +
  bot/subtype skip are the loop-prevention groundwork for it.

## Forward look

**309** — egress: a Maidan message in a linked channel/thread → Slack `chat.postMessage` (via
`bot_token`), skipping Slack-sourced messages (metadata tag). Then the Git/GitHub App projector
(310+).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 307.0]] (the
ingress foundation). Config-gated projector arc under the "both projectors, hold launch" plan.
