# Cluster 244.0 — follows/subscription foundation

> **Program C (notifications & reach), part 8** — Arc H. Phase XXIV post-gate
> hardening. Tag **`v244.0.0`**. No new gate tag.

## Goal

Open the follows half of Arc H with the zero-blast-radius foundation: the store for a
member following a channel or a thread, so the notification router can (in a later
cluster) notify followers of activity there even without a mention.

## Scope

| Change | Where |
|--------|-------|
| `maidan_channel_follows` + `maidan_thread_follows` tables (pg 0045 / sqlite 0044; PK `(member, target)`, reverse index on the target) | `migrations/*`, `migrate.rs` |
| `ChannelFollow` / `ThreadFollow` models | `maidan-types/src/models.rs` |
| Store — `follow_channel`/`unfollow_channel`/`list_channel_follows`/`channel_followers` + the thread quartet, both backends | `store.rs`, `store/{sqlite,postgres}/follows.rs`, `store/*/mod.rs` |

## Design decisions

- **Presence = following, two edge tables.** A row `(member, channel)` or
  `(member, thread)` *is* the follow — the `member_skills` / `channel_members`
  pattern. `follow_*` is an idempotent `ON CONFLICT DO NOTHING`; `unfollow_*` returns
  whether a row was removed.
- **Reverse index on the target.** The router's hot query is "who follows this
  channel/thread?" (`channel_followers` / `thread_followers`), so each table carries an
  index on the target column — the fan-out lookup stays cheap as follows grow.
- **Channel and thread follows are independent.** Following a channel is "everything
  here"; following a thread is "just this conversation". Separate tables keep the
  router's two lookups simple; a later cluster decides how they compose (a channel
  follow implying its threads is a router policy, not a schema constraint).
- **Foundation only.** Two tables + a store module; zero existing paths change
  (Cluster 230 pattern). The router doesn't read the follower sets yet.

## Non-goals / deferred

- **Router wiring** (Cluster 245) — `route_event` fans a notification to each follower
  of the event's channel/thread on activity (honoring mutes; the dedup index already
  prevents a mentioned-and-following member getting two).
- **REST** (245) + **MCP** (246) follow/unfollow/list management.

## Risks

- Migration registration — covered by the both-backend store test +
  `dialect_parity` / `concurrent_migrations`.
