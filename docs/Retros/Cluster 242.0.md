# Cluster 242.0 retro — the mute switch is wired

> Tag **`v242.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 6** — Arc H.

## What shipped

- The notification router now honors mute: `route_event` skips a `(member, kind)` the
  recipient has muted, metered as `maidan_notifications_suppressed_total{reason=muted}`.
- `PUT`/`GET /members/:id/notification-prefs` — set (upsert) / list a member's
  preferences, `workspace:read` + self-only for sessions.

The Cluster-241 foundation now does something end-to-end over REST: a member mutes a
kind, and the router stops writing those notifications.

## Surprises / decisions

- **The 241 design paid off — the router change is one guard.** Because
  `is_notification_muted` already encapsulates the "absent row → not muted" default,
  wiring it was a single `if … { record; return }` at the top of the mention arm, not
  a join + default dance in the router. Foundation-first keeps the functional cluster
  tiny.
- **Meter the suppression, not just the write.** A muted notification is a *decision*,
  so it gets its own counter (`…_suppressed_total{reason}`) rather than silently
  vanishing — operators can see how much muting is happening and why, and the reason
  label leaves room for future skip causes (follows-not-matched, presence-away).
- **Preferences are self-config → the inbox cap model.** Setting your own mute isn't
  writing workspace content, so it reuses `workspace:read` + `ensure_acting_member`
  exactly like the Cluster-239 inbox routes — a session member configures only their
  own prefs, a bearer acts as any (orchestrator). No new capability.
- **Router + REST together; MCP split off.** Shipping the guard and the 2 REST routes
  in one cluster delivers the whole mute feature over REST; the MCP tools are a clean
  follow-on (243), matching the REST-then-MCP rhythm (239 → 240).

## Capability table extension

| Change | Where |
|--------|-------|
| Router mute-check + suppressed metric; `PUT`/`GET /members/:id/notification-prefs` | `notification_router.rs`, `metrics.rs`, `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/*` |

## Risks identified + still open

- None new. The mute feature is complete over REST; MCP (243) is the remaining
  surface.

## Forward look

**243** adds the MCP `set_notification_pref` / `list_notification_prefs` tools (mute
over MCP). Then the **follows / subscription** half of Arc H (follow a channel or
thread → get notified of activity there, honoring mutes) + its REST/MCP. Then Arc I
(email/SMTP transport, digests, presence-aware routing, `/ui` center), then Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 241.0]].
