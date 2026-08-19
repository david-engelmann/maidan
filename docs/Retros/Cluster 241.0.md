# Cluster 241.0 retro — notifications get a mute switch (Arc H opens)

> Tag **`v241.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 5** — opens Arc H.

## What shipped

- `maidan_notification_prefs` (pg 0044 / sqlite 0043; PK `(member_id, kind)`,
  `muted`) + `NotificationPref` model + store `set_notification_pref` /
  `list_notification_prefs` / `is_notification_muted`, both backends. The routing
  brain's first organ — storage only; the router doesn't consult it yet.

## Surprises / decisions

- **Key mute on `EventKind`, reuse the notification vocabulary.** A notification's
  kind *is* the triggering event's kind (Cluster 237), so a mute preference keys on
  the same `EventKind` — no display-only "notification category" enum to invent or
  keep in sync. Muting `mention_recorded` is exactly "stop notifying me about
  mentions."
- **A `muted` bool, not a presence-only mute table.** The `member_skills` pattern
  (presence = the fact) would model mute as "a row means muted." But a `muted` bool
  lets `set(member, kind, false)` record an *explicit* opt-in (distinct from "no
  preference"), and leaves the table room to grow (per-kind digest cadence, delivery
  channel) without becoming a different shape. Cheap now, flexible later.
- **Put the router's question in the store.** `is_notification_muted(member, kind)`
  with its "absent row → false" default lives in the store so the router (242) adds a
  one-line guard rather than its own join + default logic. The routing brain answers;
  the router just asks.
- **Opened where every Arc has.** Table + model + store, zero wiring — the sixth
  foundation-first open of this program (159 / 217 / 226 / 230 / 234 / 237). The
  router keeps notifying unconditionally until 242, so the blast radius is nil.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_notification_prefs` + `NotificationPref` + set/list/is_muted store | `migrations/*`, `models.rs`, `store/*/notification_prefs.rs` |

## Risks identified + still open

- None — a new table off every existing path.

## Forward look

Arc H continues: **242** wire mute into the router (`route_event` skips a muted
`(member, kind)`); then **follows/subscription** (follow a channel or thread → get
notified of activity there, honoring mutes) and the **REST + MCP** management of prefs
+ follows. Then Arc I (email/SMTP transport, digests, presence-aware routing, `/ui`
center), then Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 240.0]];
opens Arc H after Arc G (per-recipient notifications) closed at 240.
