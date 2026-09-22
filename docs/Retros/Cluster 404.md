# Cluster 404 retro — member occupancy follows and manager digest

> Closing wave for Cluster 404 · target tag `v404.0.0`

Cluster 404 completes Wave 2 row #28. Cluster 387 already shipped the
run-lineage half; this cluster delivers the remaining member-occupancy follow
and manager-digest halves without turning either into an analytics system.

## What shipped

- **#943 — durable follow intent.** `maidan_member_follows` stores only the
  follower-to-followed subscription edge on both databases. Foreign-key
  cascades clean up either side; the schema rejects self-follow.
- **#944 — private-by-construction occupancy.** REST and MCP can follow,
  unfollow, list follows, and read a member's occupancy. The read combines
  ephemeral `online|away|offline` presence with assigned non-terminal threads,
  then applies `can_access_thread` before returning any item or implied count.
- **#949 — lifecycle fan-out.** Assignment, state, result, claim failure or
  expiry, and wait timeout events reach eligible followers. Owner and follower
  recipients are deduplicated; existing kind/channel/thread mutes apply; access
  is re-evaluated when the notification is materialized.
- **#947 — a gate signal that cannot fall out of sync.** Creating an approval
  gate and appending its non-federatable `approval_requested` event share one
  store transaction. The event joins the generated lexicon and exhaustive
  event contracts and supplies the digest's `gate` category.
- **#948 — one manager rollup for three surfaces.** SQLite and Postgres group
  unread per-recipient notification rows after `since` into channel buckets of
  `results`, `gates`, and `stuck`. REST, MCP, and digest email all use that
  query; workspace-level gates occupy the explicit `channel_id: null` bucket.

## What was deferred

| To | What | Why |
|----|------|-----|
| Product follow-up | A richer `/ui` presentation | The existing inbox and email surfaces consume the data; a new dashboard was not required to close the contract. |
| Maintainer | Cut `v404.0.0` | Pushing a tag triggers the release workflow and image builds. |

## Surprises

- A first capability-matrix fixture replaced `{id}`, `{nid}`, and `{sub_id}`
  but not the new `{followed_id}` placeholder. The resulting 400 looked like a
  route defect until the matrix exposed that authorization never ran.
- The MCP tool-count prose is a tested contract. Four tools in #944 moved it
  from 178 to 182, and the digest tool in #948 moved it to 183; updating only
  the final stack tip would have left the independently mergeable API PR red.
- PostgreSQL and SQLite order NULL differently by default. Stable digest
  buckets require `ORDER BY channel_id IS NOT NULL, channel_id` on both.
- Deleting #944's merged stack branch made GitHub close dependent #946 before
  it could be retargeted. The unchanged commit was rebased and reopened as
  #949; later stack bases were retained until their dependents moved to main.

## Decisions

- **Presence remains ephemeral.** The database records subscription intent,
  not presence history. Occupancy reads the live hub and therefore does not
  pretend that an online state is durable evidence.
- **An aggregate must not weaken authorization.** Filtering details while
  returning an unfiltered count still leaks private workload. The occupancy
  result is assembled only from authorized threads.
- **Delivery-time access wins over creation-time validation.** Same-workspace
  checks make a follow valid, but channel membership can change later. Every
  routed notification rechecks current thread access.
- **The digest reads notifications, not work or events.** Per-recipient rows
  already encode authorization, mute policy, deduplication, and read state.
  A parallel analytics query would have to reproduce those policies and would
  invite scope creep into worker scoring.
- **Approval creation and its signal are atomic.** A best-effort append after
  gate creation leaves a crash window in which the gate exists but no manager
  can be notified of it.

No Architecture ADR is needed: these are composition and privacy rules over
the existing presence, follow, notification, and approval primitives. The
public integration and capability maps record the wire contracts.

## Capability table extension

| Capability | First available in |
|------------|--------------------|
| Same-workspace member follows over REST and MCP | `v404.0.0` |
| Access-filtered live member occupancy | `v404.0.0` |
| Followed-member lifecycle notifications | `v404.0.0` |
| Notification-derived result/gate/stuck manager digest | `v404.0.0` |

## Risks identified + mitigated

- **Private-work inference:** occupancy items and lifecycle fan-out use the
  canonical thread-access check, including at delivery time.
- **Policy divergence:** the digest consumes the same notification rows that
  enforce mutes, authorization, and source-log deduplication.
- **Gate/event split brain:** both rows are created in one transaction on both
  database backends.
- **Transport drift:** REST, MCP, OpenAPI, capability maps, tool catalogs, and
  the documented MCP count have executable contract coverage.
- **Backend ordering drift:** explicit NULL ordering makes channel buckets
  deterministic on SQLite and Postgres.

## Risks identified + still open

- Presence is process-live state. An unavailable presence bridge reports
  `offline`; it does not synthesize historical availability.
- Existing notification retention bounds how far back a manager digest can
  reconstruct activity. That is intentional: the digest is an inbox view, not
  an audit or analytics ledger.
- The release tag remains pending maintainer action.

## Forward look

Wave 2 row #28 is complete across Cluster 387 and Cluster 404. Resume from the
ranked open items in [[Open Work]]; do not reopen lineage or expand the digest
into performance analytics as part of this close.

## Acknowledgements

#942 pinned the contract; #943, #944, #949, #947, and #948 delivered the five
implementation slices.
