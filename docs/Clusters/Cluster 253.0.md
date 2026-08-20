# Cluster 253.0 — presence-aware email routing (wiring)

> **Program C (notifications & reach), part 17** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v253.0.0`**. No new gate tag.

## Goal

Close the loop the Cluster-252 store opened: **write** a member's last-seen on
connect, and **read** it when deciding whether to send a notification email. A
member who is actively connected doesn't need an email for a notification they'll
see in-app — presence-aware routing suppresses that redundant send.

## Scope

| Change | Where |
|--------|-------|
| Touch `last_seen` on WS presence registration (best-effort, spawned — never blocks the connect) | `ws.rs` (the `register` call site) |
| `presence_skip_window_secs()` — env-gated window from `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` | `notification_router.rs` |
| Presence-aware guard in `deliver_notification_email` — skip + meter when seen within the window | `notification_router.rs` |
| `skipped_present` outcome documented on the delivery metric | `metrics.rs` |

## Design decisions

- **Touch at the WS handler, not inside `PresenceHub`.** `PresenceHub::register` is
  a synchronous, store-less in-memory structure; the store lives on `AppState`, which
  the WS handler holds. So the `touch_member_last_seen` call goes at the `register`
  call site in `ws.rs`. It is **spawned** (fire-and-forget) so a store hiccup can
  never stall a WebSocket connect, and a failure is logged, not surfaced.
- **Opt-in, zero-change by default.** `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` unset or
  `0` disables the guard — every opted-in recipient is emailed, exactly the
  Cluster-249 behaviour. A positive value (e.g. `300`) enables "skip the email if the
  recipient was seen in the last N seconds." This mirrors the project's env-gating
  convention (scheduler, retention) — a behaviour change ships dark until configured.
- **Guard runs after the address lookup.** Only a member who *would* be emailed (has
  an address on file) is a candidate to skip, so `skipped_present` counts real
  suppressions, not members who never opted in.
- **Fail-open on a read error.** If the `get_member_last_seen` lookup errors, the
  guard falls through and sends — a transient read must never silently drop a
  notification email. A negative idle (clock skew, last-seen in the future) counts as
  "active" and skips, which is the intended outcome.

## Non-goals / deferred

- Digests (scheduled unread rollups) — the next Arc-I cluster.
- Using the durable `last_seen` for anything beyond email routing (e.g. a REST
  "last active" read, roster staleness) — out of scope here.
- MCP email tools for parity — optional, low value (email is human-facing config).

## Risks

- The WS touch has no dedicated e2e (it needs a full WS client); it is a one-line
  spawn over the Cluster-252 store method (tested both backends), and the read half
  is covered by `presence_email_routing_e2e`.
- Env-global test isolation — the guard reads a process env var, so its test lives in
  its own test binary (`presence_email_routing_e2e.rs`).
