# Maidan Repository Audit — Research Corpus

**Audit date:** 2026-09-16
**Audited ref:** `main` @ `e232c73` (Cluster 399.2), per the explicit instruction to evaluate off `main`, not the latest tag.
**Third pass:** 2026-09-16 evening — re-verified against `main` @ `dab377a` (Cluster 400.5) and the bodies of PRs #905 (quickstart pin, in CI) and #906 (Cluster 387 retro, in CI).
**Fourth pass (deep dive):** 2026-09-17 — fresh clone at `main` @ `ca2ddd3`, tag `v402.0.0` confirmed via `git ls-remote`. Nine PRs merged overnight (#905–#913): D-TAG, D-1, D-2, D-3 all resolved by the maintainer. Every high-stakes claim re-verified; F-06 re-derived mechanically from `contracts/http-capability-map.json` (exactly 20 routes) and rewritten as a four-class disposition; F-07 and F-42 reframed where the deep dive found the repo already holds better patterns than the audit recommended; F-44 and F-45 added; D-5 added as a new open decision. The corpus now holds **45 findings (F-01…F-45)**.
**Audience:** the main building agent, for incorporation into its active roadmap.
**Author role:** external research and recommendations. Not directives.

## How to use this corpus

1. Start with `ROADMAP.md` for the sequenced, phased view.
2. Read `DECISIONS.md` — four of the six items are now **resolved**, with the maintainer's reasoning preserved as precedent; two (D-4, D-5) remain open and need the maintainer, not a workstream.
3. Use `FINDINGS-INDEX.md` to cross-reference every finding (F-01…F-45) against your active roadmap items — adopt, adapt, defer, or reject each on its merits. The **Status** column tells you what the repo already did about it (landed / open / reframed / resolved).
4. Each finding maps to one **initiative brief** in `initiatives/` (context, evidence, advisory recommendation, open questions, resolution signals).
5. Each major recommendation is backed by a **research note** in `research/` showing how the conclusion was reached: what was read, what was counted, what was compared.
6. `PROMPT-FOR-AGENT.md` is the handoff prompt: what to tell the building agent so it can navigate this corpus. (A copy is also pasted in the delivery message.)

## Precedence

Your active roadmap takes precedence. Nothing in this corpus instructs you to throw out a plan. Where a recommendation conflicts with something already in flight, the conflict is flagged as an **open question** in the relevant initiative brief rather than resolved here. Where the repo already fixed something the audit found (Status: landed), the brief says so — don't re-do landed work.

## Severity scale

- **P0** — Blocks safe adoption as a dependency for automated agents, or causes silently wrong behavior. Small count, by design.
- **P1** — Needs rework: real friction, broken flows, or discoverability gaps that cost builders time.
- **P2** — Nit: wording, staleness, small inconsistency. Cheap to fix, worth batching.

## Repo state as of the fourth pass (2026-09-17)

- `main` @ `ca2ddd3`; **tag `v402.0.0` cut** at that commit. Nine PRs merged overnight (#905–#913): the quickstart pin, the 387 retro, the SoD ledger + gate enforcement (#907/#908), the revocation cascade (#909), the tap per-tenant faults + cursor + scheduled verifier (#910/#911/#912), and the 400–402 retro (#913).
- D-TAG, D-1, D-2, D-3 resolved (reasoning preserved in `DECISIONS.md` as precedent); D-4 and the new D-5 remain open.
- F-23 and F-29 resolved by the tag; F-01 landed with residuals in F-03/F-04/F-05.
- Clippy clean on main at third-pass handoff; the full workspace test run status (F-43) was not re-confirmed in the fourth pass.

## Strengths to preserve

The audit found a strong core. Any roadmap work should avoid regressing these:

- **Contract discipline:** 28 event kinds, a 285-route capability map, 177 MCP tools, golden-file and bidirectional consistency tests. This is the repo's load-bearing asset.
- **Auth primitives:** capability-scoped, attenuatable tokens with a real denial matrix; `docs/Claims.md` is unusually honest about edge cases. (Now plus the #904 ratchet idiom: widening governance privilege needs `channel:admin`.)
- **CI and test depth:** secret scanning, linting, backend-parity and capability-matrix tests are serious. (Gap noted in F-43: no full-workspace run in per-PR CI.)
- **Waiter/lease model:** the waiter loop in `docs/Integration.md` and `examples/lease_demo/` are the best onboarding assets in the repo.
- **Retro discipline:** "Retro is mandatory; release tag never cut without it" (`docs/Decisions.md`) — and C5 is the measured cost of the one time it lapsed (see INIT-06). #913 followed the rule exactly: three retros written "ahead of the tag," then `v402.0.0` cut.
- **Decision style:** rejected alternatives with reasons, choke-point placement, compile-error enforcement, honest cost clauses — see `DECISIONS.md`'s resolved sections for the working examples (#907–#912).

## Corpus map

```
README.md                  — this file
ROADMAP.md                 — phased, sequenced candidate roadmap (advisory)
DECISIONS.md               — six decisions: four resolved (reasoning as precedent), two open (D-4, D-5)
FINDINGS-INDEX.md          — F-01…F-45 with severity, location, initiative, status
PROMPT-FOR-AGENT.md        — handoff prompt for the building agent
initiatives/INIT-01-…      — 11 initiative briefs (problem, evidence, advisory
                             recommendation, open questions, resolution signals)
research/R-01-…            — 8 research notes (method + evidence trails)
```

## Scope and limits

- Only `main` was evaluated (plus PR bodies for #905/#906, which were read but not merged at corpus time). Tags are behind (see R-03) and were not used as evidence.
- Findings are based on static reading of the repo at the audited commits plus cross-file consistency checks. Nothing was executed against a live server — except where the repo's own PRs report run-verified results (#905's `/health` check), which are cited as such.
- Line numbers cited are from the stated commits and will drift; file paths are the stable identifier.
- The third pass corrected three findings where the audit had misread a deliberate decision as a defect (F-22, F-23, F-29 — marked ◆ and reframed). The corpus prefers to retract loudly rather than quietly.
