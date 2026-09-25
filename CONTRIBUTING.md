# Contributing

Maidan is pre-release. The discipline below keeps the repo coherent
while it grows.

> Security issues? See [`SECURITY.md`](SECURITY.md). Do not open public
> PRs or issues for vulnerabilities.

## Local setup

```sh
git clone git@github.com:david-engelmann/maidan.git
cd maidan
make ci          # fmt, clippy, deny, test
make smoke       # docker compose up + curl /health
```

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml).

## Branch + commit conventions

- Branch names: `<kind>/<scope>-<short-slug>` where
  `kind ∈ {feat, chore, build, ci, docs, test, refactor}`.
- Commit + PR titles follow [Conventional Commits](https://www.conventionalcommits.org/).
- Squash-merge only; PR title becomes the commit on `main`.

## PR flow

Maidan is **solo-maintained.** The maintainer merges with admin rights once CI is
green, so there is no second-reviewer gate to wait on — but the bar is the CI suite,
not a rubber stamp.

1. Open a GitHub Issue from the relevant template.
2. Branch from `main` per the convention above.
3. Develop locally; `make ci` green before pushing.
4. Open a PR. Fill in the Retrospective section (mandatory).
5. All **8 required CI checks** must pass (lint, secrets scan, unit tests, integration,
   docker-compose smoke, scale-out smoke, promtool, otlp smoke). External PRs are
   reviewed by the maintainer before merge.
6. Squash-merge — the PR title + body become the commit on `main`.

## Claiming work

Two people doing the same work is the most expensive mistake a small project can
make, so claim before you start:

- Comment on the issue that you are taking it, and link your draft PR as soon as
  it exists. A claim is the comment and the open PR, not a private intention.
- One issue, one PR. If the work turns out to be two changes, open a second
  issue.
- If a claimed issue has gone quiet, ask on the issue before starting on it
  yourself.
- Cluster work (`docs/Clusters/`) is claimed by the maintainer; open an issue
  against a cluster slice rather than starting on one directly.

## What to expect

Maidan is solo-maintained, and nothing here is a service-level commitment
except security:

- **Security reports** follow [`SECURITY.md`](SECURITY.md): acknowledgement
  within 3 business days, confirmation or refutation within 10, and a default
  90-day disclosure window.
- **Issues and PRs** are handled as the maintainer's time allows. The bar for
  merging is the 8 required checks plus review, not a queue position.
- A PR that reds a required check is not merged over it. If the check is wrong,
  fixing the check is its own PR.

## Ownership

The maintainer owns every area. To find where something lives and why:

- each crate's `src/lib.rs` opens with a doc comment saying what the crate owns
  and what it defers;
- [`docs/Architecture.md`](docs/Architecture.md) maps components and data flow;
- [`docs/Decisions.md`](docs/Decisions.md) records the decisions whose
  rationale is not obvious, and a change that reverses one needs a new entry.

## Claims we do not make

[`docs/Claims.md`](docs/Claims.md) maps every load-bearing claim in the README
and on the site to a gate, a test, or an honest "not yet". The rule for
contributions follows from it:

- A sentence about what Maidan does must point at a row in `Claims.md` — or add
  one, with its evidence, in the same PR.
- No "production-ready", "secure", "1.0" or performance numbers without the gate
  or benchmark behind them. The tags are the engineering record; there is no
  marketing release.
- No internal or third-party product names in the public surface (Cluster 389):
  examples use `example.*` kinds and generic names.
- A test name says what it tests. A test that exercises a stub is not named as
  if it exercised the real service.

## How a release is cut

Tagging is the maintainer's call. The full procedure is in
[`docs/Operations.md`](docs/Operations.md); in outline:

1. The cluster's close record lands: its retro, the Capabilities and CHANGELOG
   entries, and the Roadmap pointer. `scripts/check-release-records.sh` fails
   if the README, CLAUDE.md, CHANGELOG and Capabilities disagree on the version.
2. The maintainer pushes an annotated tag `vX.0.0` from `main`.
3. `release.yml` builds binaries and multi-arch images, runs the blocking trivy
   scan, signs images and artifacts keylessly with cosign, smoke-tests the
   published image, and publishes the GitHub Release.
4. Anyone can verify a release with the `cosign` commands in the README and
   [`SECURITY.md`](SECURITY.md#verifying-a-release).

Work merged but not yet tagged is recorded in `docs/Capabilities.md` as a
source record, never as a release.

## Retrospective discipline

Every PR body includes:

```markdown
## Retrospective (PR-level)
- **What was surprising:**
- **What got deferred:**
- **What we learned:**
```

Every cluster closes with a dedicated retro PR that updates
[`docs/Retros/`](docs/Retros/), [`docs/Capabilities.md`](docs/Capabilities.md),
and [`CHANGELOG.md`](CHANGELOG.md), then cuts the release tag.

## Coding standards

- Rust 2021 edition; `rustfmt` enforced; `clippy -D warnings`.
- `thiserror` for library errors, `anyhow` only at binary boundaries.
- `tracing` for logging; no `println!` in library code.
- Tests next to code (`#[cfg(test)]`), integration tests in `tests/`.
- No comments that restate code. Comment only when the *why* is
  non-obvious.

## Secrets

- Never commit secrets. `.env`, `maidan.toml`, `*.pem`, `*.key` are
  ignored.
- CI runs a secrets scan on every PR.
- Test fixtures use synthetic values only.

## License

By contributing you agree your contributions are licensed under MIT
(see [`LICENSE`](LICENSE)).
