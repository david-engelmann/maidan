# Cluster 245.0 retro — following actually notifies

> Tag **`v245.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 9** — Arc H.

## What shipped

- The router fans `MessagePosted` to the followers of the message's channel + thread
  (minus the author, mute-aware) via a shared `notify` helper.
- `POST`/`GET /members/:id/channel-follows` + `DELETE …/:cid`, and the thread triple —
  follow/unfollow/list, self-only for sessions, follow gated on access to the target.

Following a channel or thread now does what it says: new activity there lands in the
follower's inbox.

## Surprises / decisions

- **Mention and follow can double-fire — the fix is mute, not dedup.** The forward
  note (mine, from 244) claimed the `(member, source_log_id)` dedup index prevents a
  mentioned-and-following member from getting two notifications. It doesn't: a
  `MentionRecorded` and a `MessagePosted` are *different events* with different
  `log_id`s, so the dedup can't merge them. Rather than build cross-event dedup (which
  would need per-message keys and knowing "was this member mentioned in this message"
  at message-post time, across independently-processed events), the honest answer is
  the pref system: a member for whom follow-notifications are noisy mutes
  `message_posted` and keeps mentions. Corrected the note in the docs.
- **One `notify` helper for both routing paths.** Mentions and follows both
  mute-check → dedup-insert → meter; extracting `notify(state, ws, member, kind, …)`
  means the `MessagePosted` arm is just "compute the follower set, call `notify` per
  member," and the mute/metric behavior is identical to mentions by construction.
- **Follow gates on target access.** You shouldn't be able to subscribe to a private
  channel you can't read, so `POST …/channel-follows` runs `ensure_channel_access`
  before recording the follow. The *router* doesn't re-check access at fan-out time (N
  checks per message); a follow that later loses access leaves a pointer-only
  notification, and reading the thread is still RBAC-gated — a documented follow-up.
- **Skip your own messages, skip DMs.** The author is removed from the follower set
  (no self-notify), and `dm_conversation_id.is_some()` short-circuits — the `__dm__`
  channel isn't a followable target.

## Capability table extension

| Change | Where |
|--------|-------|
| Router `MessagePosted` follower fan-out (shared `notify`); `channel-follows` + `thread-follows` REST (6 routes) | `notification_router.rs`, `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/*` |

## Risks identified + still open

- **Stale follows** (follower lost access after following) leave a pointer-only
  notification — logged as a follow-up; content reads stay RBAC-gated.

## Forward look

**246** adds the MCP follow tools (follow/unfollow/list channel + thread), closing
Arc H — preferences + subscription complete over REST + MCP. Then Arc I (email/SMTP
transport, digests, presence-aware routing, `/ui` notification center), then Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 244.0]].
