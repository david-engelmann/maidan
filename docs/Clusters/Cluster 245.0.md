# Cluster 245.0 — follows-aware router + follow REST

> **Program C (notifications & reach), part 9** — Arc H. Phase XXIV post-gate
> hardening. Tag **`v245.0.0`**. No new gate tag.

## Goal

Make the Cluster-244 follow foundation do something: the router fans a notification
to the followers of a channel/thread on new activity, and REST lets a member
follow/unfollow/list.

## Scope

| Change | Where |
|--------|-------|
| Router fans `MessagePosted` → channel + thread followers (minus the author, mute-aware) via a shared `notify` helper | `notification_router.rs` |
| `POST`/`GET /members/:id/channel-follows` + `DELETE …/:cid`, and the thread triple | `routes/member.rs`, `app.rs`, `dto.rs` |
| Full new-route preflight (OpenAPI + capability-map + matrix substitutions & body clauses) | `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **`MessagePosted` is the follow trigger.** Following a channel/thread means "tell
  me about new messages here." `route_event` gains a `MessagePosted` arm: union the
  channel's and thread's follower sets, drop the author (you don't notify yourself),
  and `notify` each. DM posts (`dm_conversation_id.is_some()`) are skipped — the
  shared `__dm__` channel isn't a followable target.
- **A shared `notify` helper.** Mention-routing and follow-routing both do
  mute-check → `create_notification_if_absent` → meter, so they share one `notify`
  helper; the mention arm calls it once, the message arm once per follower.
- **Mention + follow can both fire — and that's fine.** A `MentionRecorded` and a
  `MessagePosted` are *distinct events with distinct `log_id`s*, so a member mentioned
  in a channel they also follow gets both a mention notification and a follow one. The
  per-event dedup can't merge them (different source events). The control is
  per-kind mute: a member drowning in follow-noise mutes `message_posted` while
  keeping mentions. (The earlier plan note that dedup prevents this was wrong; the
  honest answer is mute.)
- **Follow requires access to the target.** `POST …/channel-follows` runs
  `ensure_channel_access` (thread: `ensure_thread_access`) so you can't subscribe to a
  private channel you can't read. Self-only for sessions (`ensure_acting_member`), the
  Cluster-239 model.

## Non-goals / deferred

- **MCP** follow tools (Cluster 246, closes Arc H).
- Skipping followers who've *lost* access since following (the notification is a
  pointer, and reading the thread is RBAC-gated; a stale-follow metadata pointer is a
  documented follow-up).

## Risks

- New-route preflight (6 routes) — POST bodies need matrix body clauses, and `{cid}`/
  `{tid}` need matrix substitutions; covered by `openapi_e2e` + `http_capability_matrix_e2e`.
