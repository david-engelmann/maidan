# Cluster 321.0 retro — shared glossary foundation

> Tag **`v321.0.0`**. Phase XXIV (post-gate hardening). **Cluster 3 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

A workspace's **shared glossary** — a canonical `term -> definition` (+ aliases) so
agents use words the same way. It is the anti-drift pin and the target of 319's
`defines` reference relation. This cluster is a **zero-blast-radius store foundation**
(the Cluster 159 / 217 / 234 pattern): table + model + store CRUD, both backends, **no
routes/tools yet** (those land in 322).

- **`maidan_glossary_terms`** (pg `0053` / sqlite `0052`) — `id`, `workspace_id`
  (FK→workspaces, cascade), `term`, `definition`, `aliases` (pg JSONB / sqlite TEXT JSON,
  default `[]`), `created_by` (FK→members, cascade), `created_at`, `updated_at`,
  `UNIQUE(workspace_id, term)`, `CHECK(term <> '')`, plus `idx_glossary_workspace`.
- **`GlossaryTerm` / `NewGlossaryTerm`** models (`maidan-types`) — `GlossaryTerm` is
  `Serialize`/`Deserialize` + `ToSchema` (openapi), ready for 322's wire surface.
- **Store `glossary::{set, get, list, delete}`** (both backends) + 4 trait methods:
  `set_glossary_term` upserts on `(workspace_id, term)` (overwrites definition/aliases,
  bumps `updated_at`, keeps `created_by`/`created_at`), `get`/`list` (ordered by term),
  `delete` (conditional). Reads route via `read_pool()` (replica arc); writes hit primary.

## Surprises / decisions

- **Flat by design — no hierarchy.** A glossary with broader/narrower/related edges is a
  knowledge graph, which is a *different product line*. Keeping it a flat
  term→definition+aliases map is exactly "perfect at what it does and not more" — the KG
  ambition stays out of the offering (locked anti-goal in [[Open Work]]).
- **`aliases` as a JSON column, not a child table.** Aliases are a small unordered bag read
  whole with the term; a separate `maidan_glossary_aliases` table would add a join and a
  second write for no query benefit. pg JSONB binds the `Vec<String>` directly; sqlite
  serializes to a TEXT column and `row_to_term` parses (fallible), the Cluster-173
  message-content pattern.
- **Separate table, not a `maidan_members`/`maidan_workspaces` column.** Nothing to ripple
  through a shared `row_to_X` (memory `maidan-schema-column-ripple`); the glossary is its
  own entity.
- **Upsert keyed on `(workspace_id, term)`** so re-defining a term is idempotent and
  in-place — no duplicate rows, and the original authorship/creation instant survive.

## Test evidence

`glossary` store test (both backends): set two terms, get one, missing→`None`, re-set
upserts (definition/aliases overwritten, `created_at` preserved, no duplicate row), list
ordered by term, empty term rejected by the `CHECK`, delete conditional. fmt + strict
clippy (`-D clippy::unwrap_used/expect_used`) + `--all-targets` + bootstrap-strip clean.

## Forward look

**322** surfaces the glossary over REST + MCP (CRUD) and folds it into the context pack
(so a thread's context carries the workspace's definitions). Then the rest of the arc:
confidence/conventions → as-of context replay → seed-from-message → context snapshot
artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
