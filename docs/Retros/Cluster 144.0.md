# Cluster 144.0 retro — Docs dead-link gate + latent-link cleanup

> Tag **`v144.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **Dead-link gate**: `book/book.toml` `[output.linkcheck]` renderer
  (`warning-policy = "error"`, `follow-web-links = false`); `docs.yml` installs
  `mdbook-linkcheck` and `mdbook build` now exits non-zero on a dead internal
  link. The second renderer nests HTML under `build/html/`, so the deploy
  uploads `book/build/html`.
- **35 latent broken links fixed** (all in `book/sync-docs.sh`): space-named
  files staged under hyphenated names (`Capability-Map.md`, `Agent-Integration.md`,
  `Open-Work.md`, `Cluster-A.md`); links out of the published set rewritten to
  absolute GitHub URLs; `docs/Decisions.md` stray `[`Type`]` bracket-refs fixed.
- **Backlog reconciliation**: `Remaining Work.md §4` no longer lists the global
  admin-audit API as an open gap (shipped **132**, UI **138**); the Slack-parity
  matrix + Web-UI row reflect the 134–143 `/ui` track; baselines bumped to v143.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | External/web link checking | Network flakiness; the `cargo-deny` gate already shows "green depends on time" — don't add more non-determinism. |
| n/a | Renaming source `docs/` space-files | Only the staged copies are hyphenated; the source tree keeps its Obsidian-friendly names. |
| Future | UX polish behind the §4 rows | Full emoji picker, pins panel, search facets, notification router, Workflow Builder — product, not backend. |

## Surprises

- **A "small CI add" was a 35-link cleanup.** The gate found real dead links on
  turn one — the docs job had been green for the entire life of the site while
  shipping broken links. Making the gate green *was* the cluster.
- **mdbook-linkcheck can't url-decode `%20`.** Space-named files linked with
  `%20` (which work in a browser) are false positives. Hyphenating on stage
  fixed it durably and, as a bonus, gave cleaner published URLs.
- **The linkcheck renderer nests the HTML output** (`build/` → `build/html/`)
  because a book with >1 renderer subdivides `build-dir`. Caught in the
  local build; deploy path moved accordingly.

## Decisions

- **Hyphenate on stage, don't `%20`-exclude.** Excluding would suppress real
  future breaks in those files; hyphenating removes the ambiguity entirely.
- **GitHub-rewrite out-of-set links** rather than publish more pages — keeps the
  book curated while links resolve.
- **Internal-only** checking — deterministic; the rewritten GitHub links are web
  links and aren't fetched.

## Capability table extension

| Capability | Where |
|------------|-------|
| CI fails the docs build on dead internal links (was: silently shipped) | `book/book.toml`, `.github/workflows/docs.yml`, `book/sync-docs.sh` |

## Risks identified + still open

- **Web links are not fetched** — a GitHub-rewritten URL that later 404s
  wouldn't be caught. Accepted for determinism; the high-value case (internal
  nav) is now gated.

## Forward look

The docs pipeline is now self-guarding: a future 141-style broken-nav change
fails CI instead of shipping. Remaining docs/UI work is optional UX polish.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Prompted by the
maintainer's "make the published docs perfect" ask (Cluster 141) — this closes
the loop by making CI enforce it.
