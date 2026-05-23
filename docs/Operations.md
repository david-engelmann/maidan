# Operations

How to operate the repo day-to-day. The Architecture file says *what
the system is*; this file says *what you do to it*.

> Read [`CLAUDE.md`](../CLAUDE.md) first if you have not.

## Daily commands

```sh
# Full local CI before opening any PR
cargo fmt --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace            # requires Docker for integration tests

# Run the server against in-memory SQLite (no Docker)
DATABASE_URL=sqlite::memory: cargo run --bin maidan-server

# Run the prod-style stack
docker compose --profile full up
curl http://localhost:8080/health
```

## PR flow (the long version)

### 1. Pick the next item

The cluster's plan doc (`docs/Clusters/Cluster X.md`) lists PRs in
order with the linked Issue numbers. Work them in order unless you
have a reason to swap; PR `X.N+1` is usually written assuming
`X.N` shipped.

If you are starting a new cluster, write the plan doc first (see
"Cluster kickoff" below).

### 2. Branch + commit

```sh
git checkout main
git pull --ff-only
git checkout -b <kind>/<scope>-<slug>
```

- `kind ∈ {feat, chore, build, ci, docs, test, refactor}`
- `scope` is usually a crate name (`maidan-store`) or a concept
  (`workspace-scaffold`, `cluster-c-retro`).
- `slug` is short and lowercase with dashes.

Examples: `feat/maidan-search`, `ci/release-darwin-x86`,
`docs/cluster-c-retro`.

Commit with [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(maidan-search): pgvector embeddings + semantic search
chore: governance + workspace scaffold
docs(retro): Cluster C retrospective + v0.2.0 tag prep
ci: build x86_64-apple-darwin on macos-13
```

The PR title is the commit title is the squash-merge commit title.
Make it readable as a release-notes line.

### 3. Open the PR

```sh
git push -u origin <branch>
gh pr create --base main --head <branch> --title "..." --body "..."
```

The PR body **must** follow the template in
[`docs/Conventions.md`](Conventions.md). The Retrospective section
(per-PR) is mandatory — it survives squash-merge as part of the
commit body.

The template:

```markdown
## What this PR does
<2-4 bullets>

## Linked cluster
[Cluster X — Theme](docs/Clusters/Cluster%20X.md) · Phase X.N.

## Acceptance test
<the command(s) the reviewer runs to verify green>

## Risk / rollback
<what reverts cleanly; what doesn't>

## Out of scope
<things deferred to which PR>

## Retrospective (PR-level)
- **What was surprising:** <one or two; "nothing surprising" is acceptable>
- **What got deferred:** <bullets; each links to the future PR or follow-up issue>
- **What we learned:** <if any; otherwise omit>

Closes #<issue>.
```

### 4. Watch CI

```sh
gh pr checks <num>                 # one-shot
gh pr checks <num> --watch         # watch to completion
```

Or arm a `Monitor` and keep working — the harness will notify when
checks land.

The 5 required jobs:
- `lint (fmt + clippy + deny)` — ~30s
- `secrets scan` — ~10s
- `unit tests` — ~1m
- `integration (testcontainers)` — ~1m20s
- `docker compose smoke` — ~4m

If anything goes red, fix on the branch and push again. The most
common failures and fixes are in "Debugging CI" below.

### 5. Merge

```sh
gh pr merge <num> -R david-engelmann/maidan --squash --admin --delete-branch
```

The `--admin` flag is intentional. See
[`docs/Decisions.md`](Decisions.md) for the rationale.

After merge:

```sh
git checkout main
git pull --ff-only
git branch -d <branch>
```

## Cluster kickoff

When starting cluster `X`:

1. Create labels (one-time per cluster):

   ```sh
   gh label create cluster-x --color "0e8a16" --description "Cluster X work" --repo david-engelmann/maidan
   ```

2. Create the PR-tracker issues. Each PR in the cluster's plan has
   one issue, plus an `[X.retro]` issue:

   ```sh
   gh issue create --repo david-engelmann/maidan \
     --title "[X.1] Description" \
     --label cluster-x,<area-label> \
     --body "..."
   ```

3. Add issues to the Project board:

   ```sh
   for i in <issue-numbers>; do
     gh project item-add 1 --owner david-engelmann \
       --url "https://github.com/david-engelmann/maidan/issues/$i"
   done
   ```

4. Write `docs/Clusters/Cluster X.md` with the PR ladder, ordering
   rationale, exit criteria, and risks. Use Cluster A/B/C as
   templates.

5. Update `docs/Roadmap.md`'s "Current cluster" pointer.

6. Open PR `X.1` and start the loop.

## Cluster close

When PRs `X.1` through `X.N` are merged:

1. Open the `[X.retro]` PR on branch `docs/cluster-x-retro`.

2. Create `docs/Retros/Cluster X.md` per the shape in
   [`docs/Retros/README.md`](Retros/README.md). Every section is
   mandatory:
   - What shipped (one bullet per PR, with the merge commit SHA)
   - What was deferred (table: To, What, Why)
   - Surprises
   - Decisions (link to `docs/Decisions.md` if any locked
     differently)
   - Capability table extension
   - Risks identified + mitigated
   - Risks identified + still open
   - Forward look
   - Acknowledgements

3. Update:
   - `docs/Capabilities.md` — prepend the `v0.X.0` row
   - `CHANGELOG.md` — add `[0.X.0]` section with Added / Changed /
     Removed / Fixed / Security
   - `README.md` — refresh "What's in v0.X.0" + Status line
   - `docs/Architecture.md` — refresh the "at v0.X.0" header and any
     deferred-vs-shipped subsections
   - `docs/Roadmap.md` — mark cluster complete (✓), shift "Current
     cluster" pointer to next cluster
   - `docs/Retros/README.md` — add to the index

4. Merge the retro PR.

5. **Tag the release locally first:**

   ```sh
   git checkout main
   git pull --ff-only
   git tag -a v0.X.0 -m "Cluster X: <theme>.

   <one-paragraph summary of what's in this release>

   See CHANGELOG.md [0.X.0] and docs/Retros/Cluster X.md for the full retro."
   git tag -l v0.X.0 -n20  # verify the message
   ```

   Signing: if `git config user.signingkey` is set, use `-s` instead
   of `-a`. Otherwise annotated unsigned is acceptable
   pre-1.0 — see [`docs/Decisions.md`](Decisions.md).

6. **Push the tag** — this fires
   [`.github/workflows/release.yml`](../.github/workflows/release.yml):

   ```sh
   git push origin v0.X.0
   ```

   The workflow builds:
   - `x86_64-unknown-linux-gnu` on `ubuntu-latest`
   - `aarch64-unknown-linux-gnu` on `ubuntu-latest` via `cross`
   - `aarch64-apple-darwin` on `macos-latest`
   - `x86_64-apple-darwin` on `macos-13`

   Plus multi-arch ghcr.io images:
   - `ghcr.io/david-engelmann/maidan-server:v0.X.0`
   - `ghcr.io/david-engelmann/maidan-postgres:v0.X.0`

   Plus a GitHub Release with the binaries attached.

7. Verify the Release at `https://github.com/david-engelmann/maidan/releases/tag/v0.X.0`.
   If anything failed, see "Debugging the release workflow" below.

8. Open the next cluster kickoff.

## Debugging CI

### `lint` fails

- `cargo fmt --check` failed: run `cargo fmt` locally, commit, push.
- `clippy -D warnings` failed: read the lint, fix it. If a lint is
  wrong, use `#[allow(clippy::...)]` with a `// reason: ...` comment
  explaining why.
- `cargo deny check` failed:
  - `unmaintained` advisory: if it's a dev-dep with no production
    impact, add to `deny.toml`'s `[advisories] ignore` with a
    rationale comment.
  - `wildcard` error: workspace path deps need `publish.workspace =
    true` on the crate and `publish = false` in workspace.package.
  - `vulnerability`: check if a fixed version exists; bump deps or
    ignore with rationale if the vulnerability is not reachable in
    our code path.

### `secrets` fails

trufflehog found a verified secret. Treat as a real incident:

1. Rotate the secret immediately at the issuer.
2. Force-push a history rewrite to remove it (or contact GitHub
   support if it's already on `main`).
3. Investigate how it got committed; fix the discipline gap.

If trufflehog itself is broken (the action API changed): pin to a
specific commit SHA in `ci.yml`.

### `unit tests` fails

Run `cargo test --lib --bins --workspace` locally with the same
toolchain. The toolchain pin is in `rust-toolchain.toml`; if a deep
transitive dep needs a newer rustc, bump the pin.

### `integration (testcontainers)` fails

Run `cargo nextest run --workspace --tests` locally with Docker
running. Common failures:

- "syntax error at or near `(`": a migration uses syntax that the
  testcontainer's Postgres major doesn't support. Verify the test is
  pinned to `pgvector/pgvector:pg17` (not `postgres:17-alpine`); the
  pg17 image supports everything pg16 supports plus the `vector`
  extension.
- "cannot DELETE from contentless fts5 table": the FTS5 schema was
  reverted to `content=''`. It must stay non-contentless.
- "docker unavailable": expected on CI runners without DinD. The
  test's `match Postgres::default().start().await { Err(..) => return,
  ... }` pattern handles this; if it still fails, the pattern was
  removed.

### `docker compose smoke` fails

- "wait for /health timed out": the maidan-server container didn't
  start in 120s. Check the `compose logs on failure` step output for
  why — usually a migration failure or a connection refused on
  Postgres because of healthcheck race.

If a healthcheck race recurs, increase the healthcheck retries in
`compose.yaml` or extend the `for i in 1..60` loop in `ci.yml`.

## Debugging the release workflow

If the release workflow runs but doesn't produce a GitHub Release:

1. Check the per-matrix-job status:

   ```sh
   gh run list --repo david-engelmann/maidan --workflow=release.yml --limit 5
   gh run view <run-id> --repo david-engelmann/maidan --log-failed | tail -40
   ```

2. The `github release` job is gated on `needs: [build, docker]`.
   If either is red, the release job is skipped.

3. Common failures:
   - macOS x86_64 build red on `macos-latest`: the runner is arm64
     now. Fix is `macos-13`. See PR #36.
   - Docker push fails on auth: check that the runner has
     `packages: write` permission in `release.yml`.
   - `softprops/action-gh-release` fails on
     `fail_on_unmatched_files`: one or more matrix builds didn't
     produce an artifact. Fix the matrix entry that failed.

4. To retry a release without re-tagging:

   ```sh
   gh workflow run release.yml --repo david-engelmann/maidan \
     -f tag=v0.X.0
   ```

5. To create a release manually after the workflow already failed:

   ```sh
   gh release create v0.X.0 --repo david-engelmann/maidan \
     --title "v0.X.0 — Cluster X: <theme>" \
     --notes-file <(echo "...")
   ```

## Branch protection state

`main` is protected. As of v0.2.0:

- 5 required status checks: `lint (fmt + clippy + deny)`,
  `secrets scan`, `unit tests`, `integration (testcontainers)`,
  `docker compose smoke`.
- 1 required PR review (the maintainer self-merges via `--admin`
  bypass).
- No force push.
- No deletions.
- Required conversation resolution.
- Required linear history (squash-merge only).
- `strict = true` (PR must be up-to-date with `main` before merge).

To inspect:

```sh
gh api /repos/david-engelmann/maidan/branches/main/protection | jq
```

To update (rare):

```sh
gh api -X PUT /repos/david-engelmann/maidan/branches/main/protection \
  --input <branch-protection.json>
```

A template `branch-protection.json` is generated in this session's
shell history; otherwise reconstruct from the JSON in this section.

## Project board

[Maidan Roadmap](https://github.com/users/david-engelmann/projects/1)
is the GitHub Project v2 board. Every issue gets added at creation
time via:

```sh
gh project item-add 1 --owner david-engelmann \
  --url "https://github.com/david-engelmann/maidan/issues/<num>"
```

The board has the default `Backlog` / `Planned` / `In progress` /
`In review` / `Done` columns. Moving between columns is currently
manual; future automation is a Cluster X candidate.

## When the repo is in a half-state

If something breaks mid-cluster (e.g., the user interrupts a long
session):

1. Check `git status` and `git log --oneline -10`.
2. Read the most recent retro for context.
3. Read the most recent open PR's body for what was in flight.
4. Read [`docs/Open Work.md`](Open%20Work.md) for what's queued.
5. If a branch was left uncommitted, decide:
   - Squash into a new commit and finish the PR.
   - Reset the branch (`git reset --hard origin/<branch>`) if the
     work is unwanted.

Never force-push to `main`. Branch resets are fine.
