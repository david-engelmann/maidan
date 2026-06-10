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

- Rust 2021; toolchain pinned in `rust-toolchain.toml` (currently 1.91).
- `cargo fmt --check` and `cargo clippy --all-targets --workspace -- -D warnings`
  must pass.
- `thiserror` in libraries; `anyhow` only at binary boundaries.
- `tracing` for logging — no `println!` in library code.
- Tests next to the code (`#[cfg(test)]`); integration tests in
  `tests/`; property tests via `proptest`.
- testcontainers for DB integration tests.

## Secrets

- `.env`, `maidan.toml`, `*.pem`, `*.key` are ignored.
- All credentials from env vars or external secret managers.
- CI runs a secrets scan on every PR.
- Fixtures use synthetic data only.

## CI matrix

| Job                            | Tool                       | Required |
|--------------------------------|----------------------------|----------|
| `lint (fmt + clippy + deny)`   | fmt + clippy + deny        | yes      |
| `secrets scan`                 | trufflehog                 | yes      |
| `unit tests`                   | cargo test                 | yes      |
| `integration (testcontainers)` | nextest + testcontainers   | yes      |
| `docker compose smoke`         | docker compose + curl      | yes      |
| `helm install (kind)`          | kind + helm                | no       |
| `sqlite-vec (optional feature)`| cargo test + feature flag  | no       |
| `bootstrap compile-time strip` | cargo build/test           | no       |
| `coverage (llvm-cov)`          | cargo-llvm-cov             | no       |
