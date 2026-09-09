# Cluster 360 retro — the token-budgeted context pack (Wave 1 #11, G-dev-1)

The scoped context pack (`GET /threads/:id/context`) is the product's headline
efficiency claim, but it caps message **rows** (`message_limit`), not **tokens**:
a page of wide `ContentBlock` messages within the row cap can still overflow a
model's context window — and a long middle is precisely the region a model
attends to least ("Lost in the Middle"). This cluster makes the pack
**token-budgeted**: give it a budget, and it keeps the thread's framing and its
recent tail, folds the elided middle into an auditable marker, and — for a child
task — grounds the pack in its parent's ask and decision.

## What shipped

- **360.1 (#668) — the pure primitive.** `maidan_types::pack`: `estimate_tokens`
  (`chars/4`, model-independent — promoted from the Cluster-318 `token_pack`
  evidence test, which now imports it), `message_tokens`, `PackElision` (the
  auditable elided-middle record), and `fold_messages_to_budget` — a
  framing-and-recency-preserving fold. Pure, 5 unit tests, zero wiring.
- **360.2 (#671) — REST.** `ThreadContextLimits.token_budget` +
  `ThreadContext.elision`; `GET /threads/:id/context?token_budget=N` (and the
  workspace pack's, per nested thread) fold the page before the refs/edits/
  artifacts reads, so the whole pack shrinks. Live + as-of builders; `PackElision`
  in the OpenAPI schemas.
- **360.3 (#673) — MCP.** The twin fold on `get_thread_context` /
  `snapshot_thread_context` / `get_workspace_context`, over the same
  `fold_messages_to_budget`; `out["elision"]` + the catalog schemas.
- **360.4 (#672) — child grounds.** `maidan_types::ParentGrounding` +
  `ParentGrounding::assemble` — a child thread's pack carries the parent's opening
  (framing) message + latest decision (Cluster-234 `ThreadResult`), so a fresh
  claimer knows *why it exists*. REST `ThreadContext.parent_grounding` +
  `include_parent_grounding` (default true) and the MCP twin.

## Decisions

- **The fold preserves framing + recency, not newest-N.** "Lost in the Middle"
  says a model attends to the beginning and the end; a naive newest-N truncation
  drops the opener (the task statement). The fold keeps message 0 (framing) and
  the largest recent-tail suffix that fits, elides the middle, and always keeps
  the single newest message even under an impossible budget — a pack with no
  recent message is useless, and the honest `elision` marker still says it is over
  budget.
- **The budget is for the messages array.** The glossary (323) and edit-body
  (151) levers already trim the rest of the pack; the new lever is the messages,
  which are the dominant token cost. `next_message_cursor` (row pagination) stays
  orthogonal — it pages the next *rows*; the fold shrinks the returned page's
  *tokens*.
- **One fold definition, three surfaces.** The primitive lives in `maidan_types`;
  the Cluster-318 evidence harness, the REST assembler, and the MCP pack all fold
  through it — no drift across surfaces.
- **Child grounds is safe by construction, no second auth check.** Grounding is
  produced only when the parent shares the child's **non-DM** channel — so a
  caller that passed `ensure_thread_access` on the child is, by construction,
  allowed to read the parent. The withhold rules (cross-channel / DM / tombstoned)
  live in one pure `ParentGrounding::assemble`, unit-tested, so REST and MCP can't
  drift on the safety boundary. DM channels are excluded because the shared
  `__dm__` channel is not one audience.
- **Grounding is the focused-claimer view.** Default on for a single-thread pack,
  suppressed on nested workspace-pack builds (avoids the extra reads + the
  firehose), omitted from an as-of replay (grounding reflects the parent's current
  state, not a historical point).

## Surprises

- **`serde_json::json!` borrows its values**, so the MCP builder can reference
  `&thread`/`&channel` for grounding *after* the `json!({... "thread": thread})`
  macro — no reorder or clone needed (confirmed by compile, not assumed).
- **`context_query_count_e2e` flaked once** in the non-required `coverage
  (llvm-cov)` job (small=8, large=9 — the documented connection warm-up off-by-one,
  memory `maidan-context-query-count-flake`), passed on rerun and locally. Child
  grounds adds **zero** queries for a root thread, so the guard (which uses root
  threads) is unaffected.

## Test evidence

- Types: `pack::` unit suite (estimator, within-budget/short no-fold, fold keeps
  opener+tail+records-middle, tiny-budget floor; grounding produce/withhold ×3).
- REST: `token_budget_folds_the_middle_and_records_elision`,
  `child_thread_context_grounds_on_its_parent`; `context_query_count_e2e`,
  `openapi_e2e` bijection, `as_of_replay_e2e`, `workspace_context_concurrency_e2e`
  — green.
- MCP: `token_budget_folds_the_pack_and_surfaces_elision`,
  `child_thread_pack_carries_parent_grounding`; `tools_catalog_contract` — green.

## Forward look

The pack is now token-budgeted and children are grounded. **Deferred / follow-ups
(logged in Open Work):** the other G-dev-1 sub-items — **pack-exclude-stale** (drop
stale threads from a workspace pack), **file-mediated handoff**, honoring
**AGENTS.md/GEMINI.md/CLAUDE.md** dirs + in-repo **`llms.txt`** as pack *input*
(Codex 32 KiB cap), and the pack **pushback** contract language (challenge the
framing; uncertainty is ask/stuck, not land) — each a distinct, larger design, not
folded here. Next-ranked is **Wave 1 #12 — G-dev-7** (inbound
`pull_request.merged` → `ThreadLanded`).

## Acknowledgements

Built as a five-PR run (#668 → #672/#673) on the Cluster-318 token evidence, the
Cluster-234 thread results, and the Cluster-356 parent/child threading — each
rebased onto `main` as its parent merged.
