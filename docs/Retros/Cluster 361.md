# Cluster 361 retro — the landed fact (Wave 1 #12, G-dev-7)

The GitHub projector relayed issue/PR **comments** both ways (Clusters 310–312,
346–347), but the most load-bearing fact about a task — *its PR merged, the work
landed* — never reached the room. This cluster **steals the landed fact**: an
inbound `pull_request.merged` webhook on a linked PR becomes a durable
`ThreadLanded` event, which reaches the accountable owner and followers and can be
awaited over MCP. Deliberately **not** an automation product — the fact is
recorded; the thread's FSM is not touched.

## What shipped

- **361.1 (#675) — the `ThreadLanded` vocabulary.** `EventKind::ThreadLanded` +
  `Event::ThreadLanded { workspace_id, channel_id, thread_id, repo, pr_number,
  merged_by?, merge_commit_sha?, title? }` — the full EventKind drill,
  **non-federatable** (a locally-derived projector fact, like `ThreadReady`/
  `ClaimExpired`), + the `event_kinds_contract` + `federation::remap` arms. Zero
  wiring.
- **361.2 (#678) — the projector ingress.** `POST /integrations/github/events`
  handles the `pull_request` event: a **merged** PR (`action=closed` +
  `merged=true`) **linked** to a thread publishes `ThreadLanded` on it (via
  `routes::publish`). A PR number lives in the shared issue/PR number namespace,
  so it reuses `get_github_issue_link`. Best-effort ACK; unmerged-close and
  unlinked PRs emit nothing.
- **361.3 (#677) — notification reach.** `notification_router::route_event` gains
  a `ThreadLanded` arm: the union of the thread's owner + followers is notified,
  honoring per-recipient mutes via the existing `notify` helper.
- **361.4 (#679) — `wait_for_landed`.** The MCP long-poll (the `wait_for_ready`
  analogue): block until a thread's PR lands, `thread_id`/`channel_id`-scoped,
  Cluster-354 `since_log_id` lookback, per-event `can_access_thread` filter.

## Decisions

- **"Steal the landed fact, not an automation product."** `ThreadLanded` records
  that the PR merged; it does **not** auto-transition the thread to `closed`, run
  CI, or merge anything. What the room does with the fact (transition, review,
  celebrate) stays a separate, human/agent decision. Modelled on `ThreadReady`
  (Cluster 222) — a derived reactive fact, not an FSM edge.
- **Non-federatable.** The fact is derived from *this* deployment's GitHub webhook;
  a peer must not inject a land for our threads — the same reasoning as
  `ThreadReady`/`ClaimExpired`/`ClaimFailed`.
- **A PR is an issue, number-wise.** GitHub PRs and issues share one number
  namespace, so a PR links to a thread through the existing `github_issue_links`
  table (Cluster 346) — no new link type, no migration.
- **Reach = owner ∪ followers.** The owner is the accountable party (like the
  Cluster-355 stuck notification), but owner-governance is opt-in and often unset,
  so followers make the reach meaningful. A PR merge is infrequent (not a
  `MessagePosted` hot path), so a per-recipient loop over the small set is fine —
  no batch machinery.
- **No member actor.** `merged_by` is a GitHub login string, not a `MemberId`, so
  `ThreadLanded` has no `member_id` (no accessor arm, and the notification carries
  no actor).

## Surprises

- **`event_kinds_contract` keeps its *own hardcoded* variant list** (not
  `EventKind::ALL`), so a new kind needs adding there **and** to
  `contracts/event-kinds.json` — the test's `left` (code list) was missing
  `thread_landed` until both were updated.
- **The stacked-PR cascade bit twice this cluster** (both logged before): a
  parent merged with `--delete-branch` auto-closes a child PR whose base was that
  branch, and it can't be reopened (base gone) → open a fresh PR against `main`.
  And **an uncommitted sub-cluster (361.4) rode a `git checkout`** into the wrong
  branch during the cascade — `git stash` the WIP, run the rebase, then pop it
  back on its own branch and commit. **Lesson: commit each sub-PR before starting
  the merge cascade.**
- **`clippy::redundant_locals`** flagged a `let addr = addr;` (SocketAddr is Copy)
  in a test closure — an `async move` captures a Copy value without the rebind.

## Test evidence

- Types: the event round-trip / federatable / uniqueness suite + `event_kinds_contract`.
- Server: `github_pull_request_merged_emits_thread_landed` (unmerged/unlinked →
  nothing; merged-linked → one fact with metadata),
  `router_notifies_owner_and_followers_when_a_thread_lands`.
- MCP: `wait_for_landed_returns_next_land_and_filters_private` (live / private-
  filter / lookback); both MCP contract-sync tests.

## Forward look

The landed fact is now first-class end-to-end (event → ingress → reach → wait).
**Deferred / follow-ups (logged):** a `pull_request` `reopened`/`closed`-unmerged
signal is intentionally ignored (only a merge is a land); the GitHub egress is
still comment-only (a Maidan `closed` transition does not comment "merged" back);
`ThreadLanded` does not carry a `member_id`, so a "who merged it" that maps a
GitHub login to a Maidan member is not attempted. Next-ranked is **Wave 1 #13**
(G2 / G4 / G3 / G11 — wait-edges + escalation + fair dispatch).

## Acknowledgements

Built as a five-PR run (#675 → #678/#677/#679) on the Cluster-310–312/346–347
GitHub projector, the Cluster-222 `ThreadReady` derived-fact pattern, the
Cluster-238/245 notification router + follows, and the Cluster-354 wait lookback —
each rebased onto `main` as its parent merged.
