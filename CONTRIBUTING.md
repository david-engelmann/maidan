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
make smoke       # docker compose up + curl /health (after Cluster A PR #5)
```

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml).

## Branch + commit conventions

- Branch names: `<kind>/<scope>-<short-slug>` where
  `kind ∈ {feat, chore, build, ci, docs, test, refactor}`.
- Commit + PR titles follow [Conventional Commits](https://www.conventionalcommits.org/).
- Squash-merge only; PR title becomes the commit on `main`.

## PR flow

1. Open a GitHub Issue from the relevant template.
2. Branch from `main` per the convention above.
3. Develop locally; `make ci` green before pushing.
4. Open a PR. Fill in the Retrospective section (mandatory).
5. CI must be green; one approval required.
6. Squash-merge.

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
