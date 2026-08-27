# Cluster 291.0 retro — fold the grokbot adoption/SDK pack into Open Work

> Tag **`v291.0.0`**. Phase XXIV (post-gate hardening). Docs/governance. No new gate tag.

## What shipped

The concurrent agent ("grokbot") produced a detailed adoption/SDK strategy pack —
`docs/Adoption.md` (funnel + hosted playground/cloud + client program), `docs/Clients.md`
(SDK implementation plan), `docs/Client Contract.md` (frozen v1 SDK surface),
`docs/Client Testing.md` (black-box scenarios), and a `sdk/` directory with **four
0.0.1 name-hold scaffolds** (TypeScript, Python, Rust, Go). It was left untracked and
self-labeled "new files only, do not fold into Open Work until David says to." David gave
the go.

Following the Cluster-273 pattern (fold the backlog into the single source, keep the
strategy docs as the index behind it):

- **`docs/Open Work.md`** gains an **"Adoption & ecosystem (deferred / post-launch)"**
  subsection capturing the actionable items — language SDKs (TS→Python→Go→Rust, REST+WS,
  independent SemVer, frozen v1 contract, black-box tests), the hosted playground
  (`maidan.world/play`), the hosted cloud, and an SDK interop CI — all **gated (no
  implementation without a go)**, each referencing its detailed spec doc by name.
- The **4 pack docs + `sdk/` scaffolds are committed** as the spec/index + gated
  reservations, each doc topped with a **reconciliation banner** superseding its internal
  "do not fold / new-files-only" rules and pointing at Open Work as canonical.

## Surprises / decisions

- **The grokbot went beyond docs — it scaffolded 4-language SDK code.** Committed as-is as
  gated 0.0.1 name-holds; they are **inert** for the build (the workspace `members` list is
  explicit `crates/*`, so cargo ignores `sdk/` — every build this session already ran with
  `sdk/` present on disk). `sdk/README.md`'s "do not implement without a go" stays accurate
  for the *code*; the *go* here was to fold the backlog, not to build the clients.
- **Kept the pack unpublished** (not added to `book/SUMMARY.md`/`sync-docs.sh`). Like
  `docs/Clusters/` and `docs/Post-1.0.md`, these are forward-work specs, not current
  integrator docs; publishing them would pull their cross-links + `sdk/` links into the
  mdbook linkcheck for no reader benefit. Open Work references them in backticks (not
  links), so the published Open Work stays linkcheck-clean.
- **Single source preserved.** Open Work is the one backlog; the pack is explicitly the
  spec/index behind those items, matching the CLAUDE.md governance ("Handoff.md is the
  strategy index behind those items, not a separate backlog").

## Capability table extension

Docs/governance only — no capability change.

## Risks identified + still open

- **SDK scaffolds are unverified name-holds** — real implementation + verification + CI
  when the SDK work gets a go (tracked in Open Work).
- The pack's internal cross-links assume the whole pack is present (it is, now committed).

## Forward look

Adoption/SDK work is now a tracked, gated backlog in Open Work. Remaining launch-readiness
polish (Architecture split, GitHub metadata) is unaffected.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows the [[Retros/Cluster 273.0]]
fold pattern; pack authored by the concurrent agent, reconciled here on David's go.
