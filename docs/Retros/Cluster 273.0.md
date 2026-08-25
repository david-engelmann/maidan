# Cluster 273.0 retro — strategy-pack reconciliation

> Tag **`v273.0.0`**. Phase XXIV (post-gate hardening). **Docs/governance:
> reconcile the 2026-08-25 strategy pack into the canonical backlog.** No new
> gate tag. Docs-only (no code).

## What shipped

A separate agent ("grokbot") drafted a 7-doc strategy pack (Handoff, Pre-Public
Hardening, Path to Impressive, Expansion Bets, Launch, Protocols, Providers) plus
integration edits, left uncommitted on `main`. A 5-agent review verified the content
is **tree-grounded and accurate** (no hallucinated features, no shipped-work-as-TODO,
no violated Decisions) but found three problems. This cluster keeps the pack and fixes
all three, committing it cleanly.

- **Single source of truth restored.** The pack had installed a *competing* backlog:
  Handoff.md said "do not start from Open Work/Roadmap," and CLAUDE.md/README were
  edited to enforce it. Its ID namespace (A–J, S/M/C/E/R, L1–L6) has no mapping to the
  cluster ladder / retro / tag discipline. Reverted the redirect; **Open Work.md /
  Roadmap.md remain the one canonical backlog**, and Handoff.md is reframed as the
  *strategy index* that feeds it.
- **Grokbot's scope folded into Open Work.md.** A new "Post-272 forward work" section
  is now the canonical backlog for the next program, consolidating the pack's
  genuinely-open items (each pointing to its pack doc for the "why"): the **MCP
  `2026-07-28` stateless upgrade** (headline), a **durable mail retry queue**, an **MCP
  example pack**, **client SDKs**, **Slack/Git projectors**, **pre-public cleanup
  nits**, provider recipes, and **public launch**.
- **Docs build un-broken.** The pack red the `mdbook` linkcheck gate on two things —
  a dead link to an unpublished retro (`Expansion Bets.md`) and a `- [x]` task-list
  checkbox parsed as a broken reference-link (`Providers.md`). Both fixed; a full local
  `mdbook build` (linkcheck, `warning-policy = error`) now passes with zero errors.
- **Staleness corrected.** The pack was drafted 2026-08-25 while 270–272 were in
  flight; refreshed the "v269 / in-flight" snapshots to reflect that 267–272 shipped
  (tags through `v273`), and updated Open Work's stale "latest v251" + "269–272
  remaining", CLAUDE.md's "latest v268" orientation pointer, and the Roadmap.

## Surprises / decisions

- **The `2026-07-28` MCP claim is real** — I web-verified it (the pack asserted it in 7
  docs and it looked invented since it's past my Jan-2026 cutoff and absent from the
  code). MCP did ship a stateless `2026-07-28` revision (initialize handshake removed,
  `Mcp-Method`/`Mcp-Name` headers, `Mcp-Session-Id` gone, Multi-Round-Trip Requests,
  cacheable lists). So it's **kept and promoted to the headline Open Work item**, not
  cut. The server still negotiates `2024-11-05` only (`maidan-mcp/src/server.rs:30`).
- **Empirical build beats static reasoning.** A review agent reasoned `Providers.md`
  was linkcheck-clean; the actual `mdbook build` proved otherwise (`- [x]` at line 111).
  The real build is the authority — always run it for a docs-gate question.
- **Kept, didn't discard.** The pack surfaced real value: a genuinely-scoped next
  program, plus real small cleanup findings (stale `mail.rs:5` "Not wired" comment,
  stale `extensions.rs:1` banner, outbox `list_pending` missing `FOR UPDATE SKIP
  LOCKED`, a swallowed cursor-advance in `event_stream.rs`) — all now tracked in Open
  Work.

## Capability table extension

| Change | Where |
|--------|-------|
| Strategy pack committed (7 docs) + published (SUMMARY/sync-docs already wired) | `docs/{Handoff,Launch,Providers,Protocols,Path to Impressive,Pre-Public Hardening,Expansion Bets}.md` |
| Single-source backlog restored; forward work folded in | `docs/Open Work.md`, `docs/Roadmap.md`, `CLAUDE.md`, `docs/README.md`, `docs/Handoff.md` |
| linkcheck build-breakers fixed | `docs/Providers.md`, `docs/Expansion Bets.md` |

## Risks identified + still open

- **`compose.override.yaml` is not git-ignored** (a pre-existing local dev override).
  It shows in `git status` and could be swept in by a careless `git add -A`. Not
  changed here (out of scope); worth a `.gitignore` entry later.
- The MCP `2026-07-28` upgrade is large transport-level work now tracked in Open Work —
  not started.

## Forward look

The next program is now scoped in Open Work.md's "Post-272 forward work" — pick the
arc with the maintainer (headline candidate: the MCP `2026-07-28` upgrade).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 272.0]]. Strategy pack drafted by a separate agent; reviewed and
reconciled here.
