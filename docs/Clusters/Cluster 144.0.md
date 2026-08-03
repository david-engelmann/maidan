# Cluster 144.0 — Docs dead-link gate + latent-link cleanup

**Theme:** Add a CI gate that fails the docs build on dead internal links (the
141 follow-up), fix the broken links it surfaces, and reconcile the backlog
docs against what actually shipped.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v144.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Gate (book.toml + docs.yml)** | `[output.linkcheck]` renderer (`warning-policy = error`, `follow-web-links = false`); CI installs `mdbook-linkcheck` and `mdbook build` fails on dead internal links. HTML nests under `build/html/` → deploy uploads there. |
| **Link fixes (sync-docs.sh)** | Stage space-named files under hyphenated names (kills `%20`-in-path); GitHub-rewrite links out of the published set (unpublished docs, repo files). `docs/Decisions.md` stray bracket-refs fixed. |
| **Backlog reconciliation** | `Remaining Work.md §4` + Web-UI row + baselines updated (audit API shipped 132; 134–143 UI track). |

## Why this wasn't a one-liner

Turning the gate on exposed **35** latent broken links in the *already-published*
docs — links to unpublished `docs/` pages (OIDC, Query-Tuning, …), repo source
(`crates/**/BASELINE.md`, `.github/`, `deny.toml`), and `%20`-encoded
space-filenames the checker can't resolve. The gate is only green once the docs
pass it, so fixing those **was** the cluster.

## Decisions

- **Hyphenate space-named files on stage, don't exclude `%20`.** `Capability
  Map.md` → `Capability-Map.md` in the staged copy. Durable (no
  false-positive suppression), and the published URLs get cleaner
  (`/maidan/docs/Capability-Map.html`). Source `docs/` filenames are unchanged.
- **Rewrite out-of-set links to GitHub**, not stage more pages — keeps the
  published book to the curated SUMMARY set while the links still resolve.
- **Internal-only checking** (`follow-web-links = false`) — deterministic in
  CI; the GitHub-rewritten links are web links and aren't fetched.

## Non-goals

- Checking external/web links (network flakiness; the advisories gate already
  shows "green depends on time" pain — don't add more).
- Renaming the source `docs/` files (only the staged copies are hyphenated).

## PR ladder (actual)

| # | Title |
|---|--------|
| 144.0.1 | `fix(docs): gate dead links in CI + fix 35 latent broken published links` (#378) |
| 144.0.retro | `docs(retro): Cluster 144.0 + v144.0.0 tag prep` |

## Exit criteria

- `mdbook build` fails on a dead internal link; the docs pass with 0 errors;
  deploy serves from `build/html`; backlog reconciled — **met**.
- `v144.0.0` tagged after retro.

## Verification & limits

- Local: `bash book/sync-docs.sh && mdbook build book` → exit 0, 27 pages, 0
  link errors; injected broken link → exit 101. Live site re-checked after deploy.
- Limit: only *internal* links are gated; a GitHub-rewritten URL that later 404s
  wouldn't be caught (web links are not fetched, by design).

## References

- [[Retros/Cluster 144.0]]; [[Clusters/Cluster 141.0]]; `book/sync-docs.sh`,
  `book/book.toml`, `.github/workflows/docs.yml`.
