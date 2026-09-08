# Cluster 357 retro — scoped notification mute (Wave 1 #8, N3)

Wave 1 #8 (N3). Notification mute was two-shaped: a global per-kind mute (Cluster
242, `maidan_notification_prefs`) and a per-thread leaf mute (Cluster 356.3,
`maidan_thread_mutes`). The missing middle was a **per-channel** mute — silence a
busy channel's firehose — and the policy question it raises: if you mute a
channel, should an @mention still reach you? This cluster adds per-channel mute
and answers that with **mention breakthrough**.

## What shipped

- **357.1 (#654) — per-channel mute store foundation.** `maidan_channel_mutes`
  (member, channel) table (pg 0062 / sqlite 0061, presence = muted) + store
  `mute_channel`/`unmute_channel`/`is_channel_muted`/`channel_muters` (both
  backends). Zero-blast-radius — mirrors `maidan_thread_mutes` (356.3) at channel
  granularity.
- **357.2 (#655) — router wiring + mention breakthrough + REST.** The `notify`
  path (mentions, ClaimExpired) and the `MessagePosted` fan-out now drop a
  channel-muter (`suppressed{reason="channel_muted"}`), and `POST`/`DELETE
  /channels/:cid/mute` (self-scoped, `workspace:read` + channel access). The
  headline is the **mute hierarchy**: an explicit kind-mute always wins; a thread
  mute suppresses everything incl. a mention (you're done with that thread); a
  channel mute suppresses the firehose but is **pierced by a `MentionRecorded`**
  (mute the noise, still get named).
- **357.3 (#656) — MCP parity.** `mute_channel` / `unmute_channel` — the MCP
  twins, self-scoped, channel-gated.

## Decisions

- **Side table, not a prefs-PK unification.** Open Work #8 framed N3 as "replace
  the kind-only PK `(member, kind)`". Cluster 356.3 had just committed the
  codebase to per-scope side tables (`thread_mutes` beside the kind-pref table),
  and every mute/follow surface follows that shape — so a channel mute is a
  `maidan_channel_mutes` side table, not a scope discriminator folded onto the
  241–243 prefs row. "Mute is no longer kind-only" is satisfied in spirit (it is
  now scopeable to channel + thread) without a risky migration of two
  just-shipped tables. The prefs-PK is left as-is.
- **The mute hierarchy is a total order of scopes.** kind (global) > thread
  (leaf) > channel (broad), with the mention-breakthrough carve-out applied only
  where the item asked ("a mention pierces a channel mute"). A thread mute keeps
  its 356.3 semantics (suppresses even a mention) precisely because it's the more
  specific, deliberate act; a channel mute is the coarse "too noisy" signal that a
  direct address should override.
- **Breakthrough is `MentionRecorded`-only.** A `ClaimExpired` owner-alert is a
  non-mention `notify` call, so a channel mute suppresses it like any firehose —
  a deliberate, documented scope line (a governance-breakthrough for owned tasks
  is a possible follow-up).

## Surprises

- None material — 357.1 was a near-exact mirror of the 356.3 leaf-mute
  foundation (three clusters prior, same session), and the router already had the
  thread-mute filter in both the `notify` path and the fan-out, so the channel
  filter slotted in beside it.

## Test evidence

- Store: the `follows` suite extended with channel mute/unmute/is-muted/muters
  (both backends).
- Router: `channel_mute_suppresses_firehose_but_mention_breaks_through` — the
  firehose is suppressed, a mention breaks through, then a thread mute suppresses
  even the mention.
- REST: `follows_rest_e2e::mute_and_unmute_channel` (idempotent mute; unmute 404).
- MCP: `channel_mute_tools_mute_and_unmute`; both contract-sync tests +
  `openapi_e2e` + `http_capability_matrix_e2e` green per PR.

## Forward look

**Per-channel mute + mention breakthrough are complete** over REST + MCP; with
356.3's per-thread mute, notification mute is now scopeable at kind, channel, and
thread granularity. **Deferred (N3 sub-items):** the *projector-kind overlay*
(muting notifications by their Slack/GitHub projector origin) needs the
notification to carry its projector provenance, which it doesn't cleanly today —
a separate design, logged in Open Work. Next-ranked is **Wave 1 #9 — T1 / T5**
(a tokens+USD+turns budget envelope that stops the run; an agent-work DLQ; hard
stop ≠ success).

## Acknowledgements

Built as a three-PR run (#654 → #656) on the Cluster-356.3 leaf-mute pattern and
the Cluster-238/245 notification router, each rebased onto `main` as its parent
merged.
