# Cluster 230.0 retro — agents get a place to declare what they can do

> Tag **`v230.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 14 — opens Arc E.

## What shipped

- The `maidan_member_skills` table + `MemberSkill` model + three store methods
  (add/remove/list, both backends). The skills an agent declares — storage only, no
  routing or routes yet.

## Surprises / decisions

- **Foundation-first, a fourth time.** The DAG (217), scheduler (226), and channel
  membership (159) all opened with a table + store and zero wiring, and each arc was
  calmer for it. A capability registry with routing is a genuinely new subsystem, so
  splitting the storage out keeps this PR a new-table-plus-module and isolates the
  interesting part (skill-aware claim) to its own cluster.
- **Tags, not a taxonomy.** The temptation with "skills" is a controlled vocabulary
  / ontology. The value here is purely the *match* — a task needs `code-review`, an
  agent has `code-review`, so route it — which set containment gives for free with
  free-form strings. No vocabulary to govern, no migration when a new skill appears.
- **No new typed id.** A skill isn't an entity; it's a tag on a member. The
  `(member_id, skill)` composite PK is the `channel_members` / `thread_dependencies`
  shape, with a reverse index on `skill` for the "who can do this" direction that
  231+ will need.
- **The migration checklist held (fifth time this program).** New `.sql` × 2
  backends, `const include_str!` + `apply_{pg,sqlite}(pool, N)` per backend (pg one
  ahead: 0039 / 0038), a new model, both `mod.rs` registrations. The IDE's
  missing-trait-items diagnostic flagged the impls the moment the trait grew — a nice
  fast signal before even compiling.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_member_skills` + `MemberSkill` + add/remove/list store methods | `migrations/*`, `models.rs`, `store/*/member_skills.rs` |

## Risks identified + still open

- None — a new table off every existing path.

## Forward look

Arc E builds out: **231** skill-aware claim (`thread_required_skills` + `claim_next`
only takes a task whose required skills the claimer holds), then **232** REST
management + a "capable members" discovery read, then **233** MCP tools. After Arc E:
Arc F — coordination waits + structured results (234–236). Then Programs C
(notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 229.0]].
