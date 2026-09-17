# INIT-08 — Ops and migration hygiene

**Findings:** F-31 (P2), F-32 (P1), F-33 (P2), F-34 (P2, strengthened), F-43 (P2)
**Repo state (2026-09-17):** `main` @ `ca2ddd3`. Clusters 401–402 added six migrations (pg 0097–0099, sqlite 0096–0098); the registry is now **197** `include_str!` entries, numbering still divergent (F-33 unchanged). **F-34 has its incident:** #909's author nearly shipped a migration that was never registered — the script asserted on a two-line `const` form that `cargo fmt` had collapsed to one line, "and migrations are a hardcoded `include_str!` list, so an unregistered one silently does nothing; the store test hit 'no column named parent_token_id'." The failure mode the audit predicted (a checklist item a migration author forgets) happened to the maintainer, in this exact file, this week. The author's process fix ("read the file back after scripted edits") is honest but doesn't survive the next author; only removing the hand-maintained list does.

## Problem statement

1. **Broken `make smoke` (F-32, P1).** The compose-up invocation doesn't pass a profile, so only Postgres starts; the target then waits on a server that never comes up. A broken smoke test is worse than none — it teaches contributors that `make smoke` is ornamental.
2. **Helm prod values (F-31).** `helm/maidan/values-prod.yaml` sets `pullPolicy: Always` under an otherwise empty `image:` block — no repository, no tag. Anyone deploying from these values gets surprising behavior.
3. **Migration numbering divergence (F-33).** SQLite and Postgres migration numbering diverges after 0014. Today's backend-parity tests hold, but every new migration is a chance for the two sequences to disagree about what "migration N" means.
4. **Hard-coded migration registry (F-34).** `migrate.rs` lists 191 `include_str!` entries by hand (exact count verified). It works, but it's a merge-conflict magnet and a checklist item every migration author must remember.
5. **The full workspace test run is not in per-PR CI (F-43, new — maintainer-flagged, 2026-09-16).** #900–#903 all touch `maidan-store`; each was green only against its own base. The full-workspace run — the one check that sees the four PRs together — was still in flight at handoff, after all four had merged. This is a structural gap, not a one-off: per-PR CI cannot, by construction, test a stack of store PRs against each other. Clippy on current main is clean; that is not the same check.

## Advisory recommendation

- Fix `make smoke` to pass the needed profile (or restructure the compose profiles so the default `up` is meaningful); add it to CI if it isn't there.
- Either make `helm/values-prod.yaml` genuinely production-usable (real repo/tag placeholders with documentation) or mark it explicitly as a starting template, not a working config.
- Adopt a single migration numbering authority (or a CI check that the two backends' sequences agree); **generate the `include_str!` registry with a `build.rs` that reads the migrations directory at compile time** — the standard pattern for this. This is no longer hypothetical hygiene: #909 demonstrated the exact failure (silently unregistered migration) against the current hand-maintained list. A `build.rs` makes "forgot to register" unrepresentable; the author's "read the file back" discipline does not transfer to the next author.
- Close the F-43 gap: run the full workspace test suite on a merge queue (or as a post-merge gate on `main`), so stacked PRs touching the same crate are tested together before they all land. The maintainer's observation is the specification: per-PR CI is structurally blind to cross-PR interaction in `maidan-store`.

## Open questions for the building agent

- What's the intended production deploy story — Helm, k8s overlays, compose, or Docker image? (Ties to INIT-06's F-30.) The values-prod fix depends on which path is blessed.
- Is SQLite a first-class backend or a dev convenience? The migration-numbering discipline follows from that answer.

## Signals of resolution

- `make smoke` passes from a clean checkout and runs in CI.
- `values-prod.yaml` is either working or honestly labeled a template.
- New migrations can't land with divergent numbering; the `include_str!` list isn't hand-edited.
- Stacked PRs against the same crate are tested together (merge queue or post-merge full-workspace gate) — F-43.
