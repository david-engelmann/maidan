# Cluster 141.0 — Publish the docs/* pages (fix the dead published nav)

**Theme:** The published mdBook site shipped a sidebar of ~20 dead links —
mdBook never built the `docs/*` pages because they were referenced with
`../docs/...` paths that escape its `src/` dir. Stage them in at build time so
every page publishes at a correct in-site URL.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v141.0.0`**, no new gate tag.

---

## The bug

`book/src/SUMMARY.md` linked the canonical docs as `../docs/Integration.md`,
etc. mdBook only builds chapter sources under `src/`; it **silently skipped**
the out-of-`src` entries. Result on `https://david-engelmann.github.io/maidan/`:

- Only 3 real pages existed (`introduction`, `api`, `mcp-reference`).
- Every `docs/*` sidebar link 404'd — and resolved *outside* the `/maidan/`
  base (clicking "Integrating with Maidan" → `…github.io/docs/Integration.html`,
  GitHub's user-level 404).
- The canonical integration guide was unreachable from the live nav, while the
  `docs` CI job stayed green (mdBook doesn't error on skipped entries).

## Fix

| Change | Purpose |
|--------|---------|
| `book/sync-docs.sh` (new) | Copies the 21 SUMMARY-referenced `docs/*.md` into `book/src/docs/` (generated, gitignored) so mdBook builds them. Rewrites out-of-`docs/` repo-root links (CHANGELOG/CLAUDE/contracts/rust-toolchain) to absolute GitHub URLs; flattens Obsidian `[[wikilinks]]` to plain text. |
| `book/src/SUMMARY.md`, `introduction.md`, `api.md` | Drop `../` from `docs/` links so they resolve under `/maidan/`. |
| `.github/workflows/docs.yml` | Run `book/sync-docs.sh` before `mdbook build book`. |
| `.gitignore` | Ignore generated `book/src/docs/` + `book/build/`. |
| `introduction.md` | Add a copy-pasteable local quickstart. |
| `book/src/404.md` (new) | Helpful 404 → home + integration guide; notes the `/maidan/` prefix. |

## Non-goals

- Moving `docs/` into `book/src/` — `docs/` stays the GitHub-native source of
  truth; the book stages a curated copy at build time.
- Publishing all of `docs/` (Clusters/Retros/Tracks history) — only the curated
  SUMMARY set publishes.
- Per-target `[[wikilink]]` resolution — flattened to readable plain text.

## PR ladder (actual)

| # | Title |
|---|--------|
| 141.0.1 | `fix(docs): publish the docs/* pages (every sidebar link 404'd)` (#372) |
| 141.0.retro | `docs(retro): Cluster 141.0 + v141.0.0 tag prep` |

## Exit criteria

- Every SUMMARY page builds + is reachable at an in-site URL; landing-page
  sidebar has no `../docs` escapes; live site verified after deploy — **met**.
- `v141.0.0` tagged after retro.

## Verification & limits

- Local `bash book/sync-docs.sh && mdbook build book`: warning-free, 27 pages
  (was ~6). Root sidebar links to in-site `docs/*.html` (0 `../docs` escapes);
  repo-root links resolve to GitHub; no `[[wikilinks]]` remain. Live site
  re-checked after the `main` deploy.

## References

- [[Retros/Cluster 141.0]]; `book/sync-docs.sh`, `book/src/SUMMARY.md`,
  `.github/workflows/docs.yml`.
