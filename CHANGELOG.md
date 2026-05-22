# Changelog

All notable changes to Maidan are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing yet. Next: Cluster B kickoff (routing + event bus + MCP).

## [0.0.1] — 2026-05-22

First tagged release. End of Cluster A. The repo is now a working
substrate: it builds, tests, deploys via Docker and Kubernetes, and
exposes a `/health` endpoint backed by Postgres or SQLite.

### Added

- MIT LICENSE, CONTRIBUTING.md, SECURITY.md, CHANGELOG.md,
  `.gitignore`, `.editorconfig`, `rust-toolchain.toml` (pinned to 1.88).
- Cargo workspace with 13 crates:
  `maidan-types`, `maidan-store`, `maidan-bus`, `maidan-search`,
  `maidan-fsm`, `maidan-router`, `maidan-auth`, `maidan-artifacts`,
  `maidan-mcp`, `maidan-a2a`, `maidan-observability`, `maidan-cli`,
  `maidan-server`.
- Core domain schema 0001 (workspaces, members, channels, threads,
  messages, mentions, votes, references, artifacts, audit) in both
  Postgres and SQLite dialects.
- `maidan-store::Store` async trait, `PostgresStore`, `SqliteStore`,
  `Dialect::from_url` runtime routing, idempotent migration runner.
- `maidan-artifacts::ArtifactStore` async trait, `Sha256` newtype,
  `LocalFsStore` with sha-derived fanout paths, atomic tempfile-and-
  rename writes, content-addressed dedup.
- `maidan-server`: env-driven `Config`, `AppState` over
  `Arc<dyn Trait>` handles, `/health` endpoint returning structured
  `{status, db, storage, version}` body (200 on healthy, 503 on
  degraded with per-subsystem error string), axum + tower-http
  tracing layer, migration-on-boot.
- Production multi-stage Dockerfile (cargo-chef + distroless runtime),
  `Dockerfile.dev` (cargo-watch hot reload), `docker/Dockerfile.db`
  (pgvector + bundled schema).
- `compose.yaml` (prod-style) and `compose.dev.yaml` (hot reload).
- Full Kustomize manifest set: `k8s/base/` + `overlays/dev/` +
  `overlays/prod/`.
- Obsidian docs vault under [`docs/`](docs/) with Architecture,
  Roadmap, Conventions, Deploy, Glossary, Capabilities,
  Clusters/Cluster A, Retros/Cluster A.
- Integration test suite: testcontainers-backed Postgres roundtrip,
  SQLite roundtrip (shared assertion body), cross-dialect parity
  scenario, `/health` end-to-end test, LocalFsStore roundtrip +
  concurrency stress + proptest property tests.

### Changed

- Toolchain pinned at 1.88 (forced by transitive deps `icu_*` and
  `idna`).

### Security

- Established [SECURITY.md](SECURITY.md) reporting flow (GitHub private
  advisories preferred).
- `cargo deny` allowlist + `trufflehog` scan documented in
  [`docs/Conventions.md`](docs/Conventions.md) (CI gating lands in the
  next PR).
- `k8s/base/secret.example.yaml` documents the required Secret shape
  without storing values.

[Unreleased]: https://github.com/david-engelmann/maidan/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/david-engelmann/maidan/releases/tag/v0.0.1
