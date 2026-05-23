# Cluster E retro — Artifact substrate

> Closing wave for Cluster E · target tag `v0.4.0`.

Cluster D made thread state machine-enforceable. Cluster E completes the
artifact path: S3-compatible storage, typed kinds, HTTP upload/download,
streaming ingestion, kind-aware helpers, and MCP surfaces for agents.

## What shipped

- **PR #65** — `feat(maidan-types): ArtifactKind taxonomy + migration 0007`
- **PR #66** — `feat(cluster-e): artifact substrate E.2–E.7` — S3Store,
  compose MinIO wiring, HTTP routes, `put_reader`, helpers, MCP tools +
  `maidan://artifacts/{sha}` resource; Rust **1.91** for `aws-sdk-s3`

## What was deferred

| To        | What                                              | Why                                      |
|-----------|---------------------------------------------------|------------------------------------------|
| Cluster F | Auth on artifact upload/download                  | Pre-1.0 anonymous API.                   |
| Cluster T | S3 multipart upload for very large blobs          | `put_reader` buffers; enough for v0.4.0. |
| Post-1.0  | Replace `rustls-webpki` 0.101 aws-sdk transitive  | Blocked on upstream smithy client.       |

## Surprises

- **`aws-sdk-s3` requires rustc 1.91** — toolchain bump landed in E.2.
- **Three `RUSTSEC-2026-*` advisories** on `rustls-webpki` 0.101 pulled in by
  AWS SDK; ignored with documented rationale (MinIO over HTTP in compose).
- **`CDLA-Permissive-2.0`** license from an AWS transitive crate — added to
  `deny.toml` allow list.
- **Bundled E.2–E.7 in one PR** after babysitting separate PRs proved slow;
  plan doc issue ladder still accurate for traceability.

## Decisions

- **`aws-sdk-s3` over `opendal`** — official SDK, path-style MinIO support;
  cost is MSRV + deny noise. Stays until a lighter client matures.
- **Compose `full` profile uses S3** — `minio-init` creates `maidan` bucket;
  localfs remains default for bare `cargo run` without MinIO.
- **HTTP upload via query params** — `POST /artifacts?kind=…&mime_type=…`
  with raw body; simple for agents and curl.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| `ArtifactKind` enum + DB CHECK                          | `v0.4.0`           |
| `S3Store` (MinIO-compatible)                            | `v0.4.0`           |
| `ARTIFACT_BACKEND=s3`                                   | `v0.4.0`           |
| `POST /artifacts` + `GET /artifacts/:sha`               | `v0.4.0`           |
| `put_reader` streaming helper                           | `v0.4.0`           |
| Kind-aware `put_screenshot` / … helpers                 | `v0.4.0`           |
| MCP `upload_artifact` + `get_artifact_metadata`         | `v0.4.0`           |
| MCP `maidan://artifacts/{sha256}` resource              | `v0.4.0`           |

## Risks identified + mitigated

- **Illegal artifact kinds in DB** — migration 0007 CHECK + typed `ArtifactKind`.
- **Missing MinIO bucket** — `minio-init` job in compose `full` profile.

## Risks identified + still open

- **`rustls-webpki` 0.101 advisories** — ignored in `deny.toml`; revisit when
  AWS SDK upgrades rustls.
- **Anonymous artifact API** — Cluster F auth.

## Forward look

Cluster F is auth, workspaces, and capabilities. Verify `v0.4.0` release
artifacts after retro merge.

## Acknowledgements

Solo cluster. MinIO was already in compose from Cluster A — E.3 finally wired
the server to use it.
