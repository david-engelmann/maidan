# Cluster E — Artifact substrate

After Cluster D made thread lifecycle explicit, Cluster E completes the
artifact story: production object storage, a typed kind taxonomy,
streaming I/O for large blobs, HTTP upload/download, and MCP surfaces
so agents can attach screenshots, recordings, transcripts, and code
dumps to threads.

> **Goal:** Agents and operators can store and retrieve content-addressed
> blobs via S3-compatible storage (MinIO in dev), with metadata in
> `maidan_artifacts`, `ArtifactUpserted` on the bus, and MCP tools.
>
> **Target tag:** `v0.4.0`.

## PRs

| #       | Title                                                                 | Issue |
|---------|-----------------------------------------------------------------------|-------|
| E.1     | `feat(maidan-types): ArtifactKind taxonomy + store validation`        | #56   |
| E.2     | `feat(maidan-artifacts): S3Store backend (MinIO-compatible)`          | #57   |
| E.3     | `feat(maidan-server): wire ARTIFACT_BACKEND=s3 + compose bucket init` | #58   |
| E.4     | `feat(maidan-server): artifact upload/download HTTP routes`           | #59   |
| E.5     | `feat(maidan-artifacts): streaming put/get on ArtifactStore`          | #60   |
| E.6     | `feat(maidan-artifacts): kind-aware put helpers`                      | #61   |
| E.7     | `feat(maidan-mcp): artifact tools + maidan://artifacts resource`      | #62   |
| E.retro | `docs(retro): Cluster E retrospective + v0.4.0 tag prep`              | #63   |

## Order

1. **E.1 first** — `ArtifactKind` enum (`screenshot`, `recording`,
   `transcript`, `code_dump`, `attachment`); migration 0007 adds a
   `CHECK` on both dialects; `Artifact` / `NewArtifact` use the typed
   kind; store rejects unknown strings at the boundary.
2. **E.2** — `S3Store` implementing [`ArtifactStore`] with path-style
   addressing for MinIO; keys mirror LocalFs fanout
   (`<sha[0:2]>/<sha[2:4]>/<sha[4:]>`).
3. **E.3** — `ARTIFACT_BACKEND=s3` + `S3_*` env vars (see
   [`docs/Deploy.md`](../Deploy.md)); optional `compose` init container
   or documented `mc mb` step; integration test against MinIO when
   Docker is available.
4. **E.4** — `PUT /artifacts` (body bytes + kind + mime) writes object
   store + `upsert_artifact` + publishes `ArtifactUpserted`; `GET
   /artifacts/:sha` streams body; RFC 7807 on missing body vs missing
   row.
5. **E.5** — extend trait with `put_stream` / `get_stream` (or
   equivalent); default adapters buffer for `LocalFsStore`; S3 uses
   multipart for large payloads.
6. **E.6** — `put_screenshot`, `put_transcript`, etc. set kind + default
   `mime_type` on [`NewArtifact`].
7. **E.7** — MCP `upload_artifact` / `get_artifact_metadata` tools +
   `maidan://artifacts/{sha256}` resource.
8. **E.retro** closes the cluster + cuts `v0.4.0`.

E.4 depends on E.2 only for S3 deployments; LocalFs remains the default
for SQLite dev and unit tests. E.5 can land after E.4 if HTTP still
buffers small bodies first.

## Artifact kinds (v0.4.0)

| Kind          | Typical use                          | Default MIME (helper)   |
|---------------|--------------------------------------|-------------------------|
| `screenshot`  | UI capture, PNG/WebP                 | `image/png`             |
| `recording`   | Audio/video session capture          | `application/octet-stream` |
| `transcript`  | Speech-to-text or meeting notes      | `text/plain`            |
| `code_dump`   | Patch, file bundle, log excerpt      | `text/plain`            |
| `attachment`  | Generic binary                       | `application/octet-stream` |

## Exit criteria

- CI green on `main`.
- `ARTIFACT_BACKEND=s3` works against compose MinIO; `localfs` unchanged.
- HTTP round-trip: upload bytes → metadata row → download by sha256.
- `ArtifactUpserted` visible on bus / replay API.
- Streaming path tested with a payload larger than the in-memory test
  threshold (e.g. 8 MiB) without loading full body in handler memory.
- MCP artifact tool + resource round-trip in `mcp_e2e.rs`.
- [[Retros/Cluster E]] merged.
- `v0.4.0` tagged; GitHub Release workflow produces binaries + images.

## Risks

| Risk                                                                 | Mitigation                                                                 |
|----------------------------------------------------------------------|----------------------------------------------------------------------------|
| `aws-sdk-s3` dependency weight + MSRV                                | Feature-gate `s3` in `maidan-artifacts`; document in retro.                |
| MinIO bucket missing on first `put`                                  | E.3 documents init; optional compose `minio-init` job.                   |
| SQLite `CHECK` migration on existing `kind` values                   | E.1 migration only allows known kinds; tests use valid values.             |
| HTTP upload without auth                                             | Accept for v0.4.0; Cluster F adds tokens.                                  |
| Body/metadata mismatch (row without object)                        | E.4 health + get returns 404 problem; upsert after successful `put`.       |
| Compose smoke still on `localfs`                                   | E.3 adds optional profile or post-step S3 health probe.                    |
