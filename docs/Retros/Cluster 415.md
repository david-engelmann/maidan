# Cluster 415 retro — deploys are immutable and rolling restarts are safe

> Post-gate hardening · source record (no `v415.0.0` tag yet; the tag is the maintainer's) · PRs #1021, #1022 + close record

## Outcome

A deploy names exactly what runs, a restart does not drop requests, a
release with a fixable HIGH or CRITICAL finding is not signed, and an
operator has one page to read before and during a change.

| Slice | PR | Result |
|-------|----|--------|
| 415.1 | #1021 | Helm pins by digest; an in-process shutdown drain with a grace period; trivy blocks (covering `maidan-postgres` too) and signing waits on it; quickstart and Helm prod pinned to the newest published release and checked; `cosign verify` inline in the README with an anchored identity. |
| 415.2 | #1022 | `GET /operator/status`: phase, readiness checks, search backfill, replica lag and queue depths, as JSON or a page. |

## Decisions

- **The drain is in-process.** The runtime image is distroless, so an exec
  `preStop` has no shell to run. On SIGTERM readiness fails and the listener
  stays open for `MAIDAN_SHUTDOWN_DRAIN_SECS`.
- **A skipped scan target carries its reason.** `gosu` in the Postgres image is
  skipped because its Go-stdlib findings are unreachable in a binary that
  never opens a socket; everything else is scanned, and the image takes Debian
  updates at build.
- **Pins name a published tag.** The quickstart and Helm prod pin the newest
  release that exists, not the Capabilities head, and a check fails if the
  three pins disagree or name a tag that does not exist.
- **The verify identity names the release workflow.** The old prefix match
  accepted any repository whose name starts with `maidan`.
- **`/status` reuses readiness.** One `collect()` feeds both, so they cannot
  disagree.

## What surprised us

- **Blocking trivy as written would have failed every release**, on an image
  that was not even in the scan matrix.
- **The verify identity had been loose since signing shipped**, and the
  quickstart had drifted eight releases behind.

## Carried forward

- None.
