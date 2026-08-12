# Cluster 202.0 retro — a session can only be itself now

> Tag **`v202.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 1. Opens the new four-program arc.

## What shipped

- `ensure_acting_member` — a shared guard that pins a **session** caller to its
  own member on every member-attributed write, closing a workspace-wide
  impersonation vulnerability. Bearer tokens (orchestrator model) and bypass are
  unchanged.

## Surprises / decisions

- **The guard existed — in exactly one place.** `post_message` already had the
  right check; it just was never generalized. So this cluster is less "invent a
  guard" and more "notice that eleven other handlers forgot it". Extracting the
  one-liner into `ensure_acting_member` and applying it uniformly is the whole
  fix. A grep for the guard returning a single hit was the tell.
- **Guard the actor, not the target — the one place to be careful.** A mechanical
  "add the guard wherever a member id comes from the body" would be *wrong* in two
  spots: a *mention* records the *mentioned* member (you mention others), and
  *assign* has both `actor_id` (the actor — guard) and `assignee_id` (the target —
  don't guard, you assign to others). Guarding a target would have broken
  legitimate cross-member actions. I read each handler to classify actor vs
  target rather than pattern-match the field name.
- **Bearer act-as-any is a feature, not a hole.** The orchestrator model
  deliberately lets a service token act on behalf of any member. The guard keys on
  `token_id.is_none()` (session only), so it leaves that untouched — the fix is
  specifically about *session* callers, who have a fixed human identity.
- **Testing a session needed the OIDC flow.** The unit test covers the guard's
  four branches cheaply, but proving it's *wired* on a real route needed a genuine
  session — which means the mock-OIDC login harness. I reused the `/ui` test's
  `login_session` and asserted a session reacting as another member is 403 on a
  *newly*-guarded surface (reactions), so the coverage isn't just `post_message`.

## Decisions

- **One helper, `pub(crate)` in `routes/mod.rs`**, next to `cap`/`ensure_workspace`
  — the natural home for cross-handler auth guards, reachable from `dm.rs`/
  `group_dm.rs` too.
- **Unit + one wiring e2e**, not eleven e2es. The guard is a single uniform call;
  the unit test proves the logic, one real-session e2e proves the wiring, and the
  bearer/bypass regression is covered by the existing suites staying green.

## Capability table extension

| Change | Where |
|--------|-------|
| Session-bound acting identity (`ensure_acting_member`) on all member-attributed writes | `routes/mod.rs` + write handlers |

## Risks identified + still open

- **Behavior change is session-only** — bearer/bypass unchanged, so tests and the
  orchestrator model are untouched. Open: whether a workspace bearer should be
  allowed to post into arbitrary private DMs as any participant is a policy
  question (documented, not decided). The related **subscribe/metadata** DM gap is
  the next cluster (203).

## Forward look

Program A continues: **203** DM/group-DM participation on subscribe + metadata
reads (the events/metadata analog of the Cluster 180 read gap), **204**
cross-tenant artifact isolation, **205** transactional outbox (atomic dual-write),
**206** federation ingest trust policy + an optional RLS spike. Then Programs B
(agentic orchestration), C (notifications & reach), D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens the four-program
arc from research sweep `wf_b8cdaaa2-be4`.
