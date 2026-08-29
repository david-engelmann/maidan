# Cluster 319.0 retro — typed reference relations (flagship arc, keystone)

> Tag **`v319.0.0`**. Phase XXIV (post-gate hardening). **Cluster 1 of the fidelity +
> context flagship arc** — the differentiator. No new gate tag.

## What shipped

The keystone the rest of the arc rides on: the reference edge's `relation` is now a
**controlled typed vocabulary** instead of a free string.

- **`RelationKind`** (`maidan-types`) — a controlled set `supports / refutes / defines /
  depends / duplicates / grounds / supersedes`, plus `Other(String)` so an unrecognized
  relation round-trips verbatim rather than being rejected (the SKOS-altLabel escape). The
  same subject→predicate→object shape as IBIS / W3C PROV / ClaimReview / GitHub-Linear
  relations; ~7 predicates provably suffice, so it never grows into an ontology product.
- `Reference.relation` and `NewReference.relation` are now `RelationKind` (were `String`).
  It **serializes as the bare snake_case string** on the wire (custom `Serialize`/
  `Deserialize`), so REST/MCP/event/export payloads are byte-identical to before — the
  change is type-safety + canonicalization, not a wire break.
- Both store backends (`{postgres,sqlite}/refs.rs`) bind `relation.as_str()` and parse the
  text column via `RelationKind::from_wire`; workspace import binds `as_str()` too. The
  `maidan_references` column stays `TEXT` (no migration). The `ReferenceAdded` event carries
  the typed relation automatically (it holds the `Reference`).
- REST `CreateReference` + MCP `add_reference` inputs are `RelationKind` (parsed from the
  wire string; unknown → `Other`). OpenAPI/MCP schemas still declare `relation` as
  `string` — accurate, no contract/bijection change.

## No-backwards-compat directive, applied

Per David (2026-08-29): pre-launch, backwards compat is a non-goal. So the `relation` field
changed type outright — no dual field, no shim. `From<&str>`/`From<String>` on `RelationKind`
kept the ~14 construction sites (tests, import) compiling with their existing `.into()`; two
`.to_string()` literals became `.into()`. The wire stays a string only because a bare-string
relation *is* the best design (it's what every prior-art system uses), not for compat.

## Surprises / decisions

- **Kept the wire as a bare string, not a JSON-tagged enum.** A `RelationKind` serializes to
  `"supports"`, matching every reference of the prior art — so no consumer changes and the
  OpenAPI/MCP `string` schema stays truthful. The type lives in Rust; the vocabulary is
  discipline, not a wire format.
- **Reverse-edge / by-type *queries* are deliberately the next cluster (320)**, not this
  one. 319 is the type; 320 is the "what refutes X / what supersedes Y" traversal surface
  (new store method + REST + MCP → its own new-route preflight). Foundation-first.

## Test evidence

`maidan-types` unit tests incl. new `relation_kind_tests` (controlled round-trip via
canonical snake_case; unknown → `Other` verbatim; `From` constructors); store reference
tests (`event_log`, `bulk_reads`) + `thread_context_e2e` + `openapi_e2e` bijection green;
`cargo check --workspace --all-targets` clean; strict clippy (`-D unwrap_used -D
expect_used`) + `--all-targets` + `--no-default-features` bootstrap-strip clean.

## Forward look

**320** — reverse-edge + by-type reference queries (`list_references_to` + REST/MCP): the
payoff that makes typed relations navigable. Then the rest of the arc: glossary → confidence
/ conventions → as-of context replay → seed-from-message → context snapshot artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens the fidelity + context
flagship arc ([[Open Work]]).
