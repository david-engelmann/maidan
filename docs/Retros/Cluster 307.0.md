# Cluster 307.0 retro — Slack projector ingress foundation

> Tag **`v307.0.0`**. Phase XXIV (post-gate hardening). Slack projector, part 1. No new gate tag.

## What shipped

The ingress foundation for the Slack projector (Expansion Bets, Bet 1) — a *projector*, not a
bot: it will relay between a Slack channel and a Maidan channel with **no LLM in Maidan**.
Config-gated + inert until credentials are provided:

- **`slack.rs`** — `SlackConfig::from_env` (`MAIDAN_SLACK_SIGNING_SECRET` +optional
  `MAIDAN_SLACK_BOT_TOKEN`; `None`/disabled when the signing secret is unset), a
  `verify_slack_signature` (Slack's `v0:{ts}:{body}` HMAC-SHA256 scheme, ±5-min replay window,
  constant-time compare — reusing the existing `hmac`/`sha2`/`subtle` stack) + its inverse
  `slack_signature` helper, and the `slack_events` handler.
- **`POST /integrations/slack/events`** — **unauthed** (Slack authenticates via its request
  signature, verified in-handler, not a Maidan bearer), so it's on the top-level public router
  next to `/oauth/app/token`. It returns `404` when the projector isn't configured, `401` on a
  bad/stale signature, echoes the `url_verification` challenge (the Events-API URL-setup
  handshake), and ACKs `event_callback`s (message → thread routing is 308).
- **`AppState.slack` + `attach_slack`** (the `attach_mail` config-gate pattern); wired in
  `main.rs` from `SlackConfig::from_env`.

## Surprises / decisions

- **Unauthed route, signature-authenticated.** Slack doesn't carry a Maidan bearer — the
  request signature *is* the auth. So the route lives on the public router (not behind the auth
  middleware) and verifies the signature itself; a missing/bad/stale signature is a `401`.
- **Config-gated to `404` when off.** An unconfigured deployment is byte-unchanged — the route
  exists but returns `404` until `MAIDAN_SLACK_SIGNING_SECRET` is set, mirroring the SMTP
  transport's "no config, no feature" gate. No live Slack app is needed to ship or test this.
- **Reused the HMAC stack.** `hmac`/`sha2`/`hex`/`subtle` were already in-tree (outbound webhook
  signing), so Slack's `v0` scheme needed no new dependency. Exposed a public `slack_signature`
  (the sender's inverse of verify) so tests can sign without a dev-dep duplication.
- **ACK fast on real events.** Slack retries on any non-200, so `event_callback`s ACK `200`
  immediately; the actual thread-posting (which needs the channel-link mapping) is deliberately
  308, not a slow inline path here.

## Capability table extension

New config-gated Slack ingress (`POST /integrations/slack/events`): signature verification +
the Events-API setup handshake. No LLM, no live wiring yet.

## Risks identified + still open

- Message events are ACKed but not yet routed to a Maidan thread — **308** adds the channel-link
  store + mapping and posts inbound Slack messages into Maidan. **309** is egress (Maidan →
  Slack). Live end-to-end needs the maintainer to create a Slack app + set the env secrets.

## Forward look

**308** — Slack channel-link mapping (`slack_channel_id` ↔ Maidan channel) + inbound message →
Maidan thread post. **309** — egress (a Maidan message in a linked channel → Slack
`chat.postMessage`). Then the Git/GitHub App projector (310+).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens the projector arc (Expansion
Bets 1/6) under the "both projectors, config-gated, hold launch" plan. Follows the durable-mail
arc ([[Retros/Cluster 306.0]]).
