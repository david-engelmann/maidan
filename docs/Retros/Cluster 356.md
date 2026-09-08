# Cluster 356 retro — the threading cluster (Wave 1 #7, F1 + F2 + F7)

Wave 1 #7 (F1 + F2 + F7). Maidan's collaboration surface is Slack-shaped, but a
channel's threads were a flat list with a static title and an all-or-nothing
follow. This cluster makes a thread a first-class, *titled*, navigable object: a
parent's replies collapse to per-child summaries, a post floats its thread up an
activity-ordered list, a thread can be renamed after creation, and a member can
mute one thread without leaving the channel.

## What shipped

- **356.1 (#648) — collapsed child threads (F2).** `ChildThreadSummary
  {thread, message_count}` + `Store::child_thread_summaries(parent_id)` (both
  backends, tombstoned children excluded, oldest first) + REST
  `GET /threads/:id/children`. A threaded view shows "N replies" per child
  without loading each child's messages.
- **356.2 (#649) — thread activity bump (F7).** A post now bumps its thread's
  `updated_at` in the same tx as the insert (`create_with_event` +
  `edit_with_posted_event`, both backends) + `Store::list_recently_active_threads`
  + REST `GET /channels/:cid/recent-threads` (activity-ordered). No new
  `ThreadBumped` event — `MessagePosted` already carries the `thread_id`, so the
  bump is `updated_at` + a distinct activity-ordered read, not a second event.
  SQLite normalizes the mixed `datetime('now')` / rfc3339 ordering with
  `strftime('%Y-%m-%d %H:%M:%f', updated_at)`.
- **356.3 (#650) — leaf mute (F7).** `maidan_thread_mutes` (pg 0061 / sqlite
  0060, presence = muted) + store `mute`/`unmute`/`is_thread_muted`/`thread_muters`
  (both backends) + REST `POST`/`DELETE /threads/:id/mute` (self-scoped). The
  notification router now skips a muted recipient — the `notify` path (mentions,
  the Cluster-355 ClaimExpired owner alert) and the `MessagePosted` fan-out
  (which subtracts `thread_muters` in one batch query alongside the kind-mute
  filter). Leaf mute is per-kind-independent.
- **356.4 (#651) — rename thread (F1).** `Store::set_thread_title` (both
  backends; a targeted `title` UPDATE, `NotFound` on missing/tombstoned) + REST
  `PUT /threads/:id/title` + MCP `rename_thread` (`thread:transition`; blank
  title → 400 / InvalidParams). A rename does NOT bump `updated_at` — it is
  metadata, not activity, so it must not float the thread (356.2's contract).
- **356.5 (#652) — threading MCP parity.** The MCP twins of 356.1–356.3:
  `list_child_threads`, `list_recently_active_threads`, and self
  `mute_thread`/`unmute_thread` — so an MCP-only agent has the same threading
  surface as REST.

## Decisions

- **No `ThreadBumped` event.** The Open Work item named one, but `MessagePosted`
  already carries the `thread_id` that a bump signals — a client keys its
  live-refresh (Cluster 153) on that already. So the bump is a projection
  (`updated_at` + an activity-ordered read), not a new EventKind and its 11-site
  drill + federation classification. A rename deliberately does not bump, keeping
  the activity order tied to *posts*.
- **Leaf mute keys on the thread, not the member.** The route reads
  `/threads/:id/mute` acting on the caller (`auth.member_id`), not the
  `/members/:id/…` follow shape — a personal mute has no act-on-behalf-of case.
  A separate `maidan_thread_mutes` table (mirroring `thread_follows`), not a fold
  onto the notification-prefs `(member, kind)` PK — that PK-replacement is a
  distinct future item (Open Work #8 / N3), and leaf mute is per-thread, not
  per-kind.
- **`title` stays `Option<String>` in the store, required at the edge.** The
  store `set_thread_title` mirrors `set_owner`'s `Option` (allowing a future
  clear), but REST + MCP require a non-empty (trimmed) title — a rename names the
  thread.

## Surprises

- **The 356.2 bump-ordering test flaked on SQLite `strftime('%f')` millis** — a
  same-second sub-millisecond bump tied on the second-precision create time, so
  the random-UUID id tiebreak made the order nondeterministic. Fixed by
  normalizing both sides through `strftime('%Y-%m-%d %H:%M:%f', …)` (killing the
  `'T' > ' '` rfc3339-vs-`datetime('now')` mismatch) AND making the test
  deterministic with a `sleep(15ms)` gap between the two posts.
- **`MemberId` is not `Ord`** — a mute-set assertion had to `sort_by_key(|m| m.0)`
  on the inner uuid.
- **A rebase across a squash-merge inserts the parent as a distinct commit** —
  each of 356.3/356.4/356.5 was `git rebase --onto origin/main <old-parent-sha>`
  onto the merged parent, a clean single-commit replay each time.

## Test evidence

- Store: `child_threads::run_bump_suite` (356.2), the `follows` suite extended
  with mute/unmute/muters (356.3), the `thread_owner` suite extended with rename
  (356.4) — all both backends.
- Router: `notification_router_e2e` — a leaf-muted follower gets no new
  notification from a further post.
- REST: `rename_thread_via_rest_updates_title` (trim + 400),
  `follows_rest_e2e::mute_and_unmute_thread`.
- MCP: `owner_and_steer_tools_set_get_and_clear` extended with rename;
  `threading_tools_children_recent_and_mute`; both contract-sync tests +
  `openapi_e2e` + `http_capability_matrix_e2e` green per PR.

## Forward look

**F1 + F2 + F7 are complete** over REST + MCP — every thread is a titled,
renameable object; a parent's children collapse to summaries; a post floats its
thread; a member can mute one thread. **Deferred (a stretch sub-item of #7):**
*Automerge as thread collab* — causal edits on the same titled thread. The event
log stays the log (do not CRDT the log), so this is a separate, larger design,
not folded here. Next-ranked is **Wave 1 #8 — N3** (per-thread/channel mute +
mention breakthrough + projector-kind overlay; *replace* the kind-only
notification-prefs PK `(member, kind)` of 241–243). Note that 356.3's leaf mute
already delivers N3's per-*thread* mute as a side table; N3's remaining scope is
the prefs-PK replacement + channel mute + a mention that pierces a channel mute.

## Acknowledgements

Built as a five-PR run (#648 → #652) on the Cluster-153 live `/ui` refresh, the
Cluster-238 notification router, and the Cluster-343 keyset thread pagination,
each rebased/branched onto `main` as its parent merged.
