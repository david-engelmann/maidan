# Cluster 239.0 retro — the inbox becomes readable

> Tag **`v239.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 3** — Arc G.

## What shipped

- `GET /members/:id/notifications` (list; `unread_only`, `limit`) + `GET
  …/unread-count` + `POST …/:nid/read` + `POST …/read-all` — the REST unified inbox
  over the Cluster-237 ledger, `workspace:read` + self-only for sessions.
- `mark_notification_read` is now recipient-scoped (`(member_id, id)`), so the write
  is safe-by-construction.

The 237 ledger and 238 router had no read surface; now a member (or the UI) can see
their notifications, the unread badge, and clear them.

## Surprises / decisions

- **Self-only is the right model, and the old routes don't have it.** Building the
  inbox surfaced that the *existing* mention/inbox routes (`/members/:id/mentions`,
  `/inbox`) enforce only `workspace:read` + same-workspace — so any workspace member
  can read another member's mention feed. The Cluster-202/203 hardening (session =
  self-only, bearer = act-as-any) never reached them. The new notification routes do
  it right (`ensure_acting_member`); retrofitting the legacy pair is logged as a
  follow-up rather than smuggled into this cluster.
- **Scope the mark in the store, not just the route.** The route already guards
  self-only, so a store-level `WHERE id = ?` would have been "fine". But making the
  UPDATE `WHERE id = ? AND member_id = ?` means the method can't mark someone else's
  notification *regardless of caller* — defence in depth that also gives a clean
  `404` for a foreign/missing id, at the cost of one bind. Cheap, so do it.
- **Return the badge from the mutation.** A mark-read that returns the new unread
  count saves the UI a round-trip — the same "hand back what the caller will ask for
  next" instinct behind the mutation responses elsewhere. `read-all` returns
  `{cleared}` for the same reason.
- **Bodyless POSTs dodge the preflight trap.** The memory `maidan-new-route-preflight`
  is about POST routes whose JSON extractor 422s before `cap()` can 403 — but
  mark-read and read-all carry their ids in the path and take no body, so there's no
  extractor to run first. Only the `{nid}` matrix substitution was needed; both
  contract e2es passed first try.

## Capability table extension

| Change | Where |
|--------|-------|
| `/members/:id/notifications` list + unread-count + `:nid/read` + read-all (self-only) | `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/*` |

## Risks identified + still open

- **Legacy mention/inbox routes lack self-only** (pre-existing; surfaced here) —
  logged in Open Work for a retrofit.

## Forward look

**240** closes Arc G with the MCP surface: `list_notifications` /
`mark_notification_read` / `get_unread_count` tools + a **`wait_for_notification`**
long-poll — the `wait_for_mention` generalization, now backed by the durable ledger
(not a live-only bus subscribe). Then Arc H (preferences / mute / follow) and Arc I
(email/SMTP, digests, presence-aware routing, `/ui` center).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 238.0]].
