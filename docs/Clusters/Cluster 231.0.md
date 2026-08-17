# Cluster 231.0 — skill-aware claim

> Program B (agentic orchestration), part 15. Arc E (capability registry + skill
> routing). Phase XXIV post-gate hardening. Tag **`v231.0.0`**. No new gate tag.

## Goal

Make `claim_next` route work by skill: a task's *required* skills gate who can
claim it. This is the functional core of Arc E — with it, an agent pulling the next
task only ever gets tasks it can actually do, and no new claim API is needed
(the existing REST route + MCP tool inherit it).

## Scope

| Change | Where |
|--------|-------|
| `maidan_thread_required_skills` table (pg 0040 / sqlite 0039) + `ThreadRequiredSkill` model + store `add`/`remove`/`list` | `migrations/*`, `models.rs`, `store/{sqlite,postgres}/thread_skills.rs`, `store/*/mod.rs` |
| `claim_next` / `claim_next_with_event` skill-match clause (4 SQL sites, both backends) | `store/{sqlite,postgres}/threads.rs` |

## Design decisions

- **Requirements are a claim gate, not a separate method.** The skill check is a
  `NOT EXISTS (a required skill of the candidate the claimer lacks)` clause added
  beside the Cluster-218 readiness `NOT EXISTS`. So `claim_next(channel, member)` —
  which already carried `member_id` — becomes skill-aware with no signature change,
  and the readiness + lease (Cluster 192) + skill clauses compose in one candidate
  query. A task with no required skills is claimable by anyone (the `NOT EXISTS` is
  vacuously true).
- **Postgres reuses `$1`; SQLite appends a positional bind.** The member id was
  already `$1` on Postgres, so the skill clause needs no new bind there; SQLite's
  positional `?` chain gets one more `member_id` bind at the end (a spot the readiness
  clause didn't need). Both claim sites (`claim_next` + the `_with_event` twin) got
  the identical clause via `replace_all`.
- **`set containment`, no scoring.** A task is claimable iff *every* required skill is
  one the member declared — a boolean gate, not a best-match ranking. Ranking/scoring
  is a later refinement if needed; the gate is the useful primitive.

## Non-goals / deferred

- REST/MCP to declare member skills + set thread requirements + discover capable
  members (Clusters 232–233).
- Skill *scoring* / best-fit ranking.

## Risks

- **Claim query cost.** The candidate query now has three correlated `NOT EXISTS`
  (readiness, plus the nested skill one). Task graphs + skill sets are small and both
  join columns are indexed (`idx_thread_required_skills_skill`, the member-skills PK);
  no measured concern, but noted.
