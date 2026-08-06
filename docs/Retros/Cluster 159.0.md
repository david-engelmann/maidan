# Cluster 159.0 retro — channel membership model (RBAC part A)

> Tag **`v159.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> First cluster of the flagship channel/thread RBAC arc.

## What shipped

- **`maidan_channel_members`** (postgres `0032` / sqlite `0031`):
  `(channel_id, member_id, role ∈ {member, admin}, created_at)`, PK
  `(channel_id, member_id)`, index on `member_id`.
- **`ChannelMember` / `ChannelMemberRole`** types.
- **Four `Store` methods** — `add_channel_member` (idempotent upsert, preserves
  `created_at`), `remove_channel_member`, `list_channel_members`,
  `channel_is_member` — on both backends, mirroring the `group_dm` join-table
  conventions.
- **Both-backend round-trip test** in the shared harness.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Cluster 160 | Enforcement (`ensure_channel_access`) + creator auto-add | Kept the risky flip isolated; this cluster is pure substrate. |
| Cluster 161 | `channel:admin` + membership management API | Follows enforcement. |

## Surprises

- **A GitHub Actions outage, not a code failure.** The first CI run failed
  `bootstrap-strip` + `otlp smoke` with "failed to resolve action download info"
  during *Set up job* — GitHub's action-download service flapping, before any
  compilation. It took two reruns to catch a healthy window. The real signal
  (`integration (testcontainers)`, which runs the new store code against real
  Postgres) passed on the first attempt. A reminder to read *where* a job failed
  before assuming the diff broke it.

## Decisions

- **Model-first decomposition.** Landing the table + store with no enforcement
  means Cluster 160's behavior flip rides on code that's already merged and
  tested — the enforcement PR becomes purely "call the helper," easier to review
  and revert.
- **Idempotent upsert for `add`** (preserving `created_at`) so re-adding a member
  to change their role is safe and non-duplicating — matches how membership APIs
  are typically used.

## Capability table extension

| Capability | Where |
|------------|-------|
| Channel membership model + store | `{postgres,sqlite}/channel_members.rs` |

## Risks identified + still open

- **None from this cluster** (additive, unused table). The *arc's* risk is
  concentrated in Cluster 160 (the enforcement flip + `__dm__` exemption + MCP
  `auth` threading), as flagged in the plan.

## Forward look

Cluster 160 adds `ensure_channel_access` (in a new `maidan-auth/src/access.rs`;
bypass=skip, public=workspace-open, private=explicit members, `__dm__` exempt)
across the REST + MCP content surfaces and verifies subscribe grants — landing
the helper + MCP `auth` threading with a DM-still-works e2e first, before
flipping any REST handler. Then Cluster 161 (`channel:admin` + management API).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
