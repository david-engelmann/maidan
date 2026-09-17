# INIT-06 — Docs structure and hygiene

**Findings:** F-21 (P1), F-22–F-30 (P2; F-22, F-23, F-29 reframed — see below)
**Repo state (2026-09-17):** tag `v402.0.0` cut at `ca2ddd3`; PRs #905–#913 merged. F-23 and F-29 resolved by the tag (F-44 is the one-line residual); F-22 remains reframed (wikilinks are a recorded ADR).

## Problem statement

The docs corpus is large and mostly good — which makes the structural problems stand out:

1. **The strategy pack (F-21, P1).** `docs/Launch.md`, `docs/Adoption.md`, `docs/Handoff.md` read as internal strategy memos (positioning, rollout sequencing, team handoff notes) published as public documentation. A new reader can't tell whether they're reading product docs or someone's planning notes.
2. **`[[wikilinks]]` (F-22) — reframed: not a defect.** The third pass found the ADR: `docs/Decisions.md` records "Docs vault lives in `docs/` and uses Obsidian wikilinks" as a deliberate decision — "Wikilinks degrade gracefully on GitHub (which renders them as bracketed text) without breaking the docs site. Cluster H will pick a docs generator (mdBook / Docusaurus / VitePress) and add a build pipeline that consumes the vault," with revisit slated for Cluster H. The audit's original recommendation (rewrite as relative links) is **withdrawn**: "fixing" this would mean unilaterally overriding a recorded decision. What remains is the genuine tension the finding pointed at — GitHub-first readers get dead text — but it is an *accepted* tradeoff with a revisit trigger already on the books, not an oversight. No action unless the Cluster H docs-generator decision changes the calculus.
3. **Stale snapshot headers (F-23, F-28, F-29, F-44) — three resolved, one open, one new.**
   - F-23 resolved: the tag moved. #913's "Known state" section now reads "released as `v402.0.0`"; the baseline lag was the tag gap, and the tag gap is closed. **Residual F-44 (new, P2):** the `Baseline:` header line (Open Work.md:7) still reads `v349.0.0` while the latest tag is `v402.0.0` — the retro updated "Known state" but not the header. One-line fix.
   - F-29 resolved: `CLAUDE.md:29` now reads "latest `v402.0.0`" — and the tag exists (confirmed via `git ls-remote`, 2026-09-17). The pointer is true. The underlying lesson stands (never tag syntax for a non-tag), but there is nothing left to fix.
   - F-28 stands: `docs/Threat-Model.md`'s "for Maidan v1.1.0" header is still stale.
4. **`docs/Architecture.md` API table (F-24).** The API surface table reads as a changelog dump rather than an architecture reference — it answers "what changed when" instead of "how is this organized."
5. **`docs/OIDC.md` (F-25).** Mixes a design spike with shipped state; a reader can't tell what's real. (Note the ADR at `docs/Decisions.md`: "OIDC human login deferred to `v2.0.0` (spike in `v1.4.2`)" — the doc should reflect that decision's shape.)
6. **`docs/Production.md` table hygiene (F-26).** Duplicate `MAIDAN_SESSION_SECRET` rows; the `/metrics` row sits inside the environment-variable table.
7. **`docs/Operations.md` (F-27).** Coverage-floor history is stale relative to CI's actual 40% floor, and the doc lacks a maintainer-audience banner — it reads as user docs but contains maintainer process.
8. **Deploy docs disagree (F-30).** `k8s/README.md` and `docs/Deploy.md` give different answers about the production path (`overlays/prod`).

## In CI: the Cluster 387 retro (#906) — and why doc debt is a correctness issue

PR #906 ("docs: the record Cluster 387 never got," Cluster 400.6) backfills what three Cluster-387 impl PRs (#818/#824/#828) shipped without: a table, five REST routes, four MCP tools — with **no Capabilities entry, no CHANGELOG entry, and no Roadmap paragraph**. The entries are explicitly marked "recorded late" rather than backdated.

The PR body names the cost, and it's worth carrying into this brief because it upgrades "docs hygiene" from tidiness to correctness: **C5** — `run_occupancy` computing `blocked` from the dependency DAG alone while never consulting `maidan_thread_blocks` (while `channel_occupancy` consulted both) — "sat undetected in the one surface nobody had written down." Unclaimable work read as `queued`, the number an orchestrator sizes its fleet against. Fixed in Cluster 400.1 (#900). The lesson the maintainer drew: a history that silently reads as contemporaneous is worse than one that admits the gap — and the repo's own decision log already says "Retro is mandatory; release tag never cut without it" (`docs/Decisions.md`).

Advisory consequence: the retro discipline isn't bureaucracy; C5 is the measured cost of skipping it. Any roadmap sequencing should treat "no retro" as a defect class with correctness consequences, not a docs nicety. (#906 also notes Wave 2 #28 is still not closed — follow-a-member occupancy and the manager digest remain — and that land-gate enforcement is still "stated unconditionally in four docs" while third-party/human-ness is unenforced; that last item overlaps this brief's F-21/F-24 territory and is still open.)

## Advisory recommendation

- **Decide what the strategy pack is.** Either rewrite Launch/Adoption/Handoff as public-facing docs (positioning, adoption guide, contributor handoff) or move them under a clearly-marked internal directory (e.g., `docs/internal/strategy/`) with a header stating they're planning artifacts, not product docs. The current middle state serves neither reader.
- **Do not "fix" the wikilinks** (F-22, reframed). If GitHub-first reading ever matters enough to revisit, the venue is the Cluster H docs-generator decision, not a drive-by link rewrite.
- F-44: bump the Open Work.md `Baseline:` header to `v402.0.0` (the retro updated "Known state" but missed the header line).
- Replace other hand-maintained version pointers in doc headers with generated ones or remove them; a header that can't stay true shouldn't make claims. (F-23 resolved via the tag; F-29 resolved the same way.)
- Give `docs/Operations.md` an audience banner ("maintainer runbook, not user docs") and reconcile the k8s/Deploy disagreement by picking one production path.
- Protect the retro discipline that #906 restores: no cluster closes without its record (the repo's own ADR already says this — the work is enforcement, not a new rule).

## Open questions for the building agent

- Is there an intended docs information architecture (tutorials / how-tos / reference / explanation), or did the corpus grow organically? The strategy-pack decision is easier inside an IA.
- Are Launch/Adoption/Handoff still *accurate* as strategy, or are they stale as well as misplaced? That determines rewrite vs relocate.
- Should agent-facing docs (`CLAUDE.md`, `docs/Integration.md`) be generated or tested against the contract sources (event kinds, capability map) so they can't drift? (F-29 is a concrete drift instance.)
- The "land-gate stated unconditionally in four docs" item from #906 — is that this brief's work or the security track's? It sits at the docs/code-boundary.

## Signals of resolution

- No version header makes claims automation doesn't maintain (`CLAUDE.md` fixed; Open Work baseline moves when D-TAG moves).
- Strategy pack is either public-grade or clearly internal.
- One coherent production-deploy story across `k8s/README.md`, `docs/Deploy.md`, and Helm values.
- Every cluster from 387 forward has its retro/Capabilities/CHANGELOG record (#906's backfill holds; no new gaps open).
