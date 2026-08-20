# Cluster 249.0 — wire email delivery into the router

> **Program C (notifications & reach), part 13** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v249.0.0`**. No new gate tag.

## Goal

Connect the pieces: when the notification router writes a per-recipient notification,
also deliver it by email to members who have an address on file — if an SMTP
transport is configured. The payoff of the Cluster-247 transport + Cluster-248
address store.

## Scope

| Change | Where |
|--------|-------|
| `AppState.mail: Option<Arc<dyn MailTransport>>` + `attach_mail` builder | `state.rs` |
| `main.rs` builds the transport from `SmtpConfig::from_env` and attaches it | `main.rs` |
| Router `deliver_notification_email` — spawned best-effort after a notification write; `maidan_email_delivered_total{outcome}` metric | `notification_router.rs`, `metrics.rs` |

## Design decisions

- **Config-gated end to end.** `AppState.mail` is `None` unless `main.rs` built a
  transport from `MAIDAN_SMTP_*`; `AppState::new` (tests/embedders) always leaves it
  `None`. So email only happens in a server binary with SMTP configured — the same
  gate as Cluster 247, now reaching the router.
- **Presence of an address = opt-in.** A member is emailed iff `get_member_email`
  returns an address (Cluster 248). Setting an address opts in; deleting it opts out.
  No separate email-on flag for the MVP (per-kind email vs in-app is a later
  refinement).
- **Spawned, best-effort — never block routing.** The router is a single serial bus
  consumer; awaiting a slow/failing SMTP send inline would stall *all* notification
  processing (including the in-app writes). So the send is `tokio::spawn`ed after the
  in-app notification is written, and a failure is logged + metered
  (`maidan_email_delivered_total{outcome=failed}`), not retried. A durable retrying
  delivery queue (the webhook-worker pattern) is the documented follow-up.
- **`deliver_notification_email` is a `pub` awaitable unit.** The spawn is just prod
  scheduling; the function is called directly (awaited) by the test with a recording
  transport, so the "who gets emailed" logic is verified deterministically without a
  spawn race.
- **`attach_mail`, not a `new` param.** Mirrors `attach_resource_notifier` /
  `attach_presence_notifier` — adds the field with a `None` default in `new`, so none
  of the many `AppState::new` call sites change.

## Non-goals / deferred

- **REST/MCP to set a member's address** (Cluster 250) — today it's set via the store
  (248) only, so this cluster is wired but the address surface is next.
- A durable retrying email-delivery queue; a richer HTML/template email body;
  per-kind email routing. Digests, presence-aware routing, `/ui` center (rest of Arc I).

## Risks

- The send is best-effort (no retry) — acceptable for the MVP and documented; the
  metric surfaces failures. Blocking-the-router was the real hazard, avoided by
  spawning. `--no-default-features` still compiles (bootstrap-strip).
