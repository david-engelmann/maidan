# Cluster 253.0 retro — don't email someone who's already here

> Tag **`v253.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 17** — Arc I.

## What shipped

- **Write:** the WS `/ws/subscribe` handler now `touch`es `maidan_member_last_seen`
  on presence registration (best-effort, spawned — never blocks the connect).
- **Read:** `deliver_notification_email` gained a presence-aware guard —
  `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` (opt-in; unset/0 = off) suppresses the email
  when the recipient was seen within the window, metered as
  `maidan_email_delivered_total{outcome="skipped_present"}`.

The Cluster-252 store foundation is now wired end-to-end: presence writes on connect,
routing reads on send.

## Surprises / decisions

- **The touch belongs at the WS handler, not in `PresenceHub`.** The obvious home for
  "record last-seen on presence registration" is `PresenceHub::register` — but that
  hub is a synchronous, store-less in-memory structure by design (it fans ephemeral
  frames; it never touches Postgres). The store hangs off `AppState`, which the WS
  handler holds. So the durable touch goes at the `register` call site in `ws.rs`,
  spawned so it can't add latency (or a failure mode) to a connect.
- **Opt-in, not opt-out.** Presence-aware suppression is a genuine behaviour change —
  a deployment that currently emails on every notification would suddenly go quiet for
  active users. Defaulting it *off* (unset/0 window) keeps Cluster-249 behaviour
  byte-for-byte until an operator sets a window, matching how the scheduler and
  retention sweepers ship dark. The feature is a knob, not a new default.
- **Fail-open.** A `get_member_last_seen` read error falls through to *send*. The cost
  of a spurious extra email is trivial; the cost of silently swallowing a notification
  because a read blipped is not. Same instinct as the address-lookup path.
- **Seconds granularity + negative-idle-safe.** The window compares
  `signed_duration_since(last_seen).num_seconds() < window_secs`, so a clock-skewed
  future `last_seen` (negative idle) still reads as "active" and skips — no `to_std()`
  error path to get wrong.

## Capability table extension

| Change | Where |
|--------|-------|
| Touch `last_seen` on WS presence registration | `ws.rs` |
| Presence-aware guard + `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` in `deliver_notification_email` | `notification_router.rs`, `metrics.rs` |

## Risks identified + still open

- The WS touch has no standalone e2e (would need a full WS client) — it's a one-line
  spawn over the both-backend-tested Cluster-252 store method; the read half is proven
  by `presence_email_routing_e2e`.

## Forward look

Arc I remaining: scheduled **digests** (unread rollups emailed on a cadence — reuse
the Cluster-227 scheduler-sweeper shape), and — optionally — MCP email tools for
parity. Then **Program D (scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 252.0]].
