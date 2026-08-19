# Cluster 244.0 retro — follow a channel or thread

> Tag **`v244.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 8** — Arc H.

## What shipped

- `maidan_channel_follows` + `maidan_thread_follows` (pg 0045 / sqlite 0044; presence =
  following, reverse index on the target) + `ChannelFollow` / `ThreadFollow` models +
  the eight store methods (follow / unfollow / list / followers, per target), both
  backends. The subscription substrate — storage only; the router doesn't read it yet.

## Surprises / decisions

- **Presence-as-follow, two edge tables.** Following is a binary fact, so a row *is*
  the follow (the `member_skills` / `channel_members` shape) — no `following` bool.
  `follow_*` is idempotent, `unfollow_*` reports whether it removed anything. The
  cheapest thing that models it.
- **The reverse index is for the router, not the member.** A member listing their own
  follows is rare and small; the *hot* query is the router asking "who follows this
  channel?" on every routed event. So each table indexes the target column — the
  fan-out set is a cheap index scan, not a table scan, as follows scale.
- **Channel and thread follows stay separate.** They answer different questions
  ("everything in #incidents" vs "this one thread"), and keeping two tables leaves the
  compose question (does a channel follow imply its threads?) to the router as a
  policy in 245, rather than baking it into the schema now.
- **Sixth-plus foundation-first open — muscle memory.** Two tables + a store module +
  zero wiring; the router keeps its Cluster-242 behavior (mentions only, mute-aware)
  until 245 reads the follower sets.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_channel_follows` + `maidan_thread_follows` + `ChannelFollow`/`ThreadFollow` + store follow/unfollow/list/followers | `migrations/*`, `models.rs`, `store/*/follows.rs` |

## Risks identified + still open

- None — new tables off every existing path.

## Forward look

**245** wires follows into the router — `route_event` fans a notification to each
follower of the event's channel/thread on activity (honoring the Cluster-242 mute
check; the dedup index prevents a mentioned-and-following member getting two) — plus
the REST follow/unfollow/list management. **246** adds the MCP tools, closing Arc H.
Then Arc I (email/SMTP transport, digests, presence-aware routing, `/ui` center), then
Program D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 243.0]].
