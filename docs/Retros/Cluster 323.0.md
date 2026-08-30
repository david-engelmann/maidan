# Cluster 323.0 retro — glossary in the context pack

> Tag **`v323.0.0`**. Phase XXIV (post-gate hardening). **Cluster 5 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The grounding payoff: a workspace's glossary now rides the context pack, so an agent's
context is grounded in the shared vocabulary **without a second call**.

- **REST** — `GET /threads/:id/context` and `GET /workspaces/:wid/context` now carry a
  `glossary` field (`Vec<GlossaryTerm>`). New `include_glossary` query param, **default
  `true`**; the field is `skip_serializing_if = "Vec::is_empty"`, so a workspace with no
  glossary sees a byte-identical response and a token-tight caller can pass
  `include_glossary=false`.
- **MCP** — the `get_thread_context` / `get_workspace_context` tools attach `glossary` the
  same way (JSON builder in `maidan-mcp/src/context.rs`), with the matching
  `include_glossary` arg (default true) surfaced in the catalog.
- **Dedup on the workspace pack.** `build_workspace_context` fetches the glossary **once**
  at the top level and builds each nested thread context with `include_glossary=false`, so
  it is not repeated N times across a page of threads.

## Surprises / decisions

- **Default-on, empty-omitted** — the deliberate inversion of the Cluster-151/152/178
  lean-by-default convention, and the right call here: the glossary is the flagship's
  grounding feature, it is *curated and bounded* (not activity-scaled like messages/edits),
  and `skip_serializing_if` makes default-on **free** for the common empty case. You only
  pay tokens once someone has curated a glossary — exactly when you want it.
- **The query-count invariant held.** Adding `list_glossary_terms` to `build_thread_context`
  is one **constant** query per pack, independent of message count, so
  `context_query_count_e2e` (which asserts `large == small`, not an exact number) stayed
  green untouched.
- **Full-body-vs-metadata split didn't apply** — a glossary term is small (term +
  definition + a few aliases), so there is no lean/heavy variant; it is all-or-nothing.
- **No new store method.** Reused `Store::list_glossary_terms` from 321; this cluster is
  pure assembly + wiring.

## Test evidence

`glossary_context_e2e` (REST: default-on carries the term, `include_glossary=false` drops
it, workspace pack carries it once at the top and nested threads don't repeat it);
`context::tests::context_carries_the_glossary_and_dedups_in_workspace_pack` (MCP twin);
`context_query_count_e2e` + `thread_context_e2e` + `workspace_context_concurrency_e2e` +
`context_pagination_e2e` + `openapi_e2e` bijection unchanged/green. fmt + strict clippy +
`--all-targets` + bootstrap-strip clean; mdbook linkcheck green.

## Forward look

The glossary layer (321 store → 322 REST/MCP → 323 context fold) is **complete**. Next in
the arc: **optional `confidence` + near-zero-code conventions** (a decision-record shape
over `thread_results` + the `supersedes` edge; an `ack` grounding act) → as-of context
replay → seed-from-message → context snapshot artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
