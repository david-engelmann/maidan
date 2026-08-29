# Cluster 320.0 retro — reverse-edge + by-type reference queries

> Tag **`v320.0.0`**. Phase XXIV (post-gate hardening). **Cluster 2 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The traversal payoff for 319's typed relations: you can now query the reference graph
**backwards** ("what references this?") and **by relation type** ("what refutes X?").

- **`Store::list_references_to(dst_kind, dst_id)`** (both backends) — the reverse edge,
  reusing the **already-existing `idx_references_dst` index** (no migration). Mirrors
  `list_references_from`.
- **REST `GET /references` reshaped** — `ListReferencesQuery` now takes *either* the
  `src_kind+src_id` pair (forward) *or* the `dst_kind+dst_id` pair (reverse), plus an
  optional `relation` filter (a `RelationKind` wire string). Exactly one pair required
  (else `400`); access gated on that anchor entity (the Cluster-165 model), symmetric to
  `create_reference`. Same route + cap → no new-route preflight.
- **MCP `list_references` tool** (new) — the same src-or-dst + relation query. MCP had
  `add_reference` but **no way to list references at all** before this. Standard 5-place
  wiring (`workspace:read`, shares `add_reference`'s src/dst access-gate loop, dispatch,
  catalog schema, both sorted contracts). Also enriched the `add_reference`/`list_references`
  catalog `relation` descriptions with the controlled set.

## Surprises / decisions

- **No migration, no new route.** The `idx_references_dst` index shipped back in the core
  migration (`0001`), and reshaping the existing `GET /references` query (rather than adding
  `GET /references/to`) meant no capability-map/matrix/OpenAPI-path churn — the matrix test
  already drives `?src_kind=…&src_id=…`, which still works.
- **Relation filter is in-memory** on the returned set (the volume is small and the anchor
  is already indexed), not a third DB query variant — simpler and the ratio is tiny.
- **Access model stays anchor-gated**, symmetric to forward listing: `list_references_to`
  gates on the *target's* access and returns all inbound edges (the src ids aren't
  access-filtered — matching how forward listing doesn't filter dst; references are
  workspace-scoped metadata gated on the anchor).

## Test evidence

`references_reverse_e2e` (reverse returns both edges; `relation=refutes` filters to one;
forward still works; neither-pair → `400`); `mcp_e2e` (incl. contract-sync for the new
`list_references` tool) + `http_capability_matrix_e2e` (reshaped route) + `openapi_e2e`
bijection green; fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Next in the arc: **shared glossary / definitions layer** (the `defines` edge's target; the
anti-drift pin), then confidence/conventions → as-of context replay → seed-from-message →
context snapshot artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
