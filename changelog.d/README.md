# Changelog fragments

Each change adds one file here instead of editing `CHANGELOG.md`, so two PRs
never touch the same lines. When five lanes edited the end of `[Unreleased]`,
every merge conflicted every other open PR, and an entry from a PR branched
before a release record merged into the released section (#1488, fixed by
#1511).

**Write one.** Name the file after your branch, without the kind prefix
(`feat/claim-wait` → `changelog.d/claim-wait.md`), and write the section
exactly as it will read in the CHANGELOG:

```markdown
### A claimant waits for work in the claim itself

- **Added:** `claim_next_workspace_thread` takes `wait_ms` …
```

One `### ` heading, then `- **Added:**`, `- **Changed:**`, `- **Fixed:**`,
`- **Removed:**`, `- **Security:**` or `- **Deprecated:**` bullets. No `## `
headings. `bash scripts/changelog-fragments.sh check` (in the `changelog
(released sections)` job) holds every file to that shape.

**At a release,** `bash scripts/changelog-fragments.sh release X.0.0` moves the
`[Unreleased]` entries and every fragment, oldest first, into a new
`## [X.0.0] — <today>` section and deletes the fragments. The release record PR
then writes the section's summary paragraph. `preview` prints what
`[Unreleased]` would hold.

This README is not a fragment.
