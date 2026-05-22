# Conventions

How work flows through the repo.

## Branches

`<kind>/<scope>-<short-slug>` where:

- `kind ∈ {feat, chore, build, ci, docs, test, refactor}`.
- `scope` matches the relevant Conventional Commits scope, often a crate
  name (e.g. `maidan-store`).

Examples:

- `chore/governance-bootstrap`
- `feat/maidan-store-postgres`
- `feat/maidan-server-health`
- `docs/cluster-a-retro`

## Commit + PR titles

[Conventional Commits](https://www.conventionalcommits.org/). The PR
title becomes the squash commit on `main`, so it must read well as
release notes.

Examples:

- `chore: governance + workspace scaffold`
- `feat(maidan-store): postgres impl + schema 0001`
- `feat(maidan-server): /health endpoint + compose.yaml`

## PR body template

```markdown
## What this PR does

<2–4 bullets>

## Linked cluster

[[Clusters/Cluster A]] · Phase A.<N>

## Acceptance test

<the command(s) the reviewer runs to verify green>

## Risk / rollback

<what reverts cleanly; what does not>

## Out of scope

<things deferred and to which PR>

## Retrospective (PR-level)

- **What was surprising:**
- **What got deferred:**
- **What we learned:**
```

The Retrospective section is mandatory. Squash-merge preserves it in the
commit body so each merged commit carries its own retro.

## Code

- Rust 2021; toolchain pinned in `rust-toolchain.toml`.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
  must pass.
- `thiserror` in libraries; `anyhow` only at binary boundaries.
- `tracing` for logging — no `println!` in library code.
- Tests next to the code (`#[cfg(test)]`); integration tests in
  `tests/`; property tests via `proptest`; snapshot tests via `insta`.
- testcontainers for DB integration tests.

## Secrets

- `.env`, `maidan.toml`, `*.pem`, `*.key` are ignored.
- All credentials from env vars or external secret managers.
- CI runs a secrets scan on every PR.
- Fixtures use synthetic data only.

## CI matrix

| Job                  | Tool                       | Required | First required in |
|----------------------|----------------------------|----------|-------------------|
| `lint`               | fmt + clippy + deny        | yes      | PR #2             |
| `secrets`            | trufflehog                 | yes      | PR #2             |
| `test`               | cargo test                 | yes      | PR #2             |
| `integration`        | nextest + testcontainers   | yes      | PR #3             |
| `integration-sqlite` | nextest + sqlite           | yes      | PR #6             |
| `coverage`           | cargo-llvm-cov             | yes      | PR #3             |
| `e2e-smoke`          | docker compose + curl      | yes      | PR #5             |
| `mutation` (nightly) | cargo-mutants              | no       | —                 |
| `bench` (nightly)    | criterion                  | no       | —                 |
