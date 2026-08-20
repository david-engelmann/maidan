# Cluster 249.0 retro — notifications reach the inbox (the email inbox)

> Tag **`v249.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 13** — Arc I.

## What shipped

- `AppState.mail` (an optional `MailTransport`) + `attach_mail`, wired from
  `SmtpConfig::from_env` in `main.rs`.
- `deliver_notification_email` — the router, after writing a per-recipient
  notification, spawns a best-effort email send to the member's address (when a
  transport is configured and they have one), metered by
  `maidan_email_delivered_total{outcome}`.

The 247 transport + 248 address store are now connected: with `MAIDAN_SMTP_*` set, a
member who's registered an email gets their notifications delivered off-platform.

## Surprises / decisions

- **Never await SMTP inside the router.** The single most important call: the
  notification router is a serial bus consumer, so awaiting a slow or hanging SMTP
  send would stall *every* notification — in-app writes included — behind one bad
  mail server. The email send is `tokio::spawn`ed after the in-app row is written, so
  routing latency is unchanged and a dead SMTP host degrades to "emails don't arrive,"
  not "notifications stop." The cost is no retry (best-effort, logged + metered); a
  durable delivery queue like the webhook worker's is the honest follow-up.
- **Extract the awaitable, spawn the wrapper.** A spawned task is a nightmare to test
  deterministically (poll-with-timeout races). Splitting out
  `deliver_notification_email(state, member, kind, log_id)` as a plain `pub async fn`
  — which `notify` spawns in prod but the test awaits directly with a recording
  transport — gives a race-free assertion of exactly who gets emailed.
- **Address-presence is the opt-in.** No new "email me" flag: a member is emailed iff
  they have an address row (248). Setting an address opts in, deleting opts out — the
  minimal control surface, and finer per-kind email routing can layer on later.
- **`attach_mail`, not a `new` parameter.** `AppState::new` has ~10 positional args
  and many callers (every e2e). Following the `attach_*` setter pattern (field
  defaulted `None` in `new`, set only by the binary) meant zero call-site churn and
  keeps tests email-free by construction.

## Capability table extension

| Change | Where |
|--------|-------|
| `AppState.mail` + `attach_mail`; `main.rs` SMTP wiring; router `deliver_notification_email` + metric | `state.rs`, `main.rs`, `notification_router.rs`, `metrics.rs` |

## Risks identified + still open

- **Best-effort, no retry** — a transient SMTP failure drops that email (logged +
  metered). A durable retrying queue is the follow-up.
- **No address surface yet** — set via the store only until Cluster 250 adds REST/MCP.

## Forward look

**250** adds REST + MCP to set/get/clear a member's delivery address (so a member can
opt in without direct store access). Then scheduled digests + unread rollups,
presence-aware routing (needs durable `last_seen`), and the `/ui` notification center.
Then Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 248.0]].
