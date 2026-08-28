# Cluster 309.0 retro — Slack projector egress (arc closer)

> Tag **`v309.0.0`**. Phase XXIV (post-gate hardening). Slack projector, part 3 / finale. No new gate tag.

## What shipped

The egress half — a Maidan message in a linked thread now appears in Slack — completing the
bidirectional Slack projector (307 ingress foundation → 308 links + inbound → 309 egress):

- **`SlackSender` trait + `SlackWebClient`** (`slack.rs`) — `post_message(channel, text)` via the
  Slack Web API `chat.postMessage` (bearer `bot_token`; treats HTTP-200-`{"ok":false}` as an
  error). `SlackError` (`thiserror`). A trait so tests inject a mock.
- **`route_message_to_slack(state, thread_id, message)`** — no-op unless a `SlackSender` is
  configured; **skips messages that originated in Slack** (the `metadata.slack` tag from 308's
  ingress) so a projected inbound message is never echoed back (loop prevention); resolves the
  thread's Slack channel via the new store `get_slack_channel_link_by_thread` and relays the
  body. Best-effort, metered `maidan_slack_egress_total{outcome}`.
- **Hooked into the existing notification-router bus consumer** — the `MessagePosted` arm now
  calls `route_message_to_slack` (after the DM skip). No new bus subscription / reconnect
  boilerplate; the always-on consumer already sees every event.
- **`AppState.slack_sender` + `attach_slack_sender`**; `main.rs` attaches a `SlackWebClient` when
  `MAIDAN_SLACK_BOT_TOKEN` is set (ingress works without one).

## Surprises / decisions

- **Reused the notification-router consumer, not a parallel one.** A second bus consumer would
  duplicate ~50 lines of subscribe/reconnect/shutdown machinery. Extracting the egress *decision*
  into `slack.rs` and calling it from the router's `MessagePosted` arm is one line + keeps the
  Slack logic in `slack.rs`. Coupling is minimal and the call is a cheap no-op without a sender.
- **Loop prevention completes here.** Two guards close the loop: ingress skips `bot_id` messages
  (307/308) so our egress echo never re-enters, and egress skips `metadata.slack` messages so an
  ingested Slack message never bounces back out. The e2e proves both directions.
- **Sender is a trait for testability.** Like `MailTransport`, the mock `SlackSender` lets the
  egress logic (relay / skip-slack-sourced / skip-unlinked) be asserted end-to-end without a live
  Slack; the real `chat.postMessage` HTTP call is request-built + verified but only exercised live.

## Capability table extension

Slack egress: a Maidan message in a linked thread → Slack `chat.postMessage`; loop-safe both
ways. **Completes the bidirectional Slack projector (307–309).** No new capability/route.

## Risks identified + still open

- Egress is best-effort (a failed `chat.postMessage` is logged + metered, not retried) — a
  durable Slack outbox is a possible follow-up, but a dropped relay is low-harm (unlike a lost
  notification email, which has the mail outbox).
- No `thread_id` index on `maidan_slack_channel_links` (small table; the reverse lookup scans) —
  a follow-up if link volume ever grows.

## Forward look

The Slack projector is complete (bidirectional, config-gated, loop-safe). Next: the **Git /
GitHub App projector** (310+) — GitHub App webhook → thread → issue/PR comment + Check Run. Then
hold at the launch gate.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Completes the Slack projector opened at
[[Retros/Cluster 307.0]]. Config-gated; live wiring needs a Slack app + `MAIDAN_SLACK_*` secrets.
