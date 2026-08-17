# Cluster 230.0 — capability-registry foundation (member skills)

> Program B (agentic orchestration), part 14. Opens **Arc E — capability registry
> + skill routing**. Phase XXIV post-gate hardening. Tag **`v230.0.0`**. No new
> gate tag.

## Goal

Open the capability-registry arc with a **zero-blast-radius foundation**: the store
+ model for the skills an agent declares, and nothing wired in yet. Mirrors how the
DAG (217), scheduler (226), and channel membership (159) started — land the table +
store first, so the next clusters (skill-aware claim, REST, MCP) build on a tested
base.

## Scope

| Change | Where |
|--------|-------|
| `maidan_member_skills` table (pg 0039 / sqlite 0038), registered in `migrate.rs` | `migrations/{postgres,sqlite}/`, `migrate.rs` |
| `MemberSkill` model | `maidan-types/src/models.rs` |
| 3 store methods, both backends: `add` (idempotent) / `remove` (conditional) / `list` | `store.rs`, `store/{sqlite,postgres}/member_skills.rs`, `store/*/mod.rs` |

## Design decisions

- **Skills are free-form string tags.** Like channel names or slash-command names —
  an agent declares `["rust", "code-review"]`. No controlled vocabulary; the value
  is the routing (set containment), not a taxonomy.
- **`(member_id, skill)` composite PK, no new typed id.** Skills aren't entities;
  they're tags on a member (the `channel_members` / `thread_dependencies` shape). A
  reverse index on `skill` supports "who has this skill" for routing/discovery.
- **Empty skill rejected at the store** (`CHECK (skill <> '')` + an `InvalidInput`
  guard in `add`), like `thread_deps::add`'s self-loop guard — a cheap invariant the
  foundation owns.
- **Foundation only.** No worker, no routes, no events — the blast radius is a new
  table + a new store module. Zero existing code paths change.

## Non-goals / deferred (the rest of Arc E)

- **Skill-aware claim** (Cluster 231): `thread_required_skills` + extend `claim_next`
  so a task is only claimable by a member holding all its required skills (one
  `NOT EXISTS` clause beside the readiness clause).
- **REST management + discovery** (232) and **MCP tools** (233).

## Risks

- None — a new table + store module, off every existing path.
