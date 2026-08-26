# Cluster 278.0 retro — a real one-command quickstart

> Tag **`v278.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P0:
> five-minute quickstart.** No new gate tag.

## What shipped

A genuine one-command path from a clean machine to two agents collaborating, without a
Rust toolchain:

```sh
docker compose -f compose.quickstart.yaml up -d --build
./scripts/quickstart-two-agents.sh
```

- **`docker/Dockerfile.quickstart`** pulls a **pinned, SHA-256-verified** `v277.0.0`
  release binary (per-arch, `amd64`/`arm64`) onto `ubuntu:24.04` (matching the release
  builder's glibc), runs **non-root**, and pre-chowns `/data` so a fresh named volume
  inherits the ownership (SQLite + localfs write without a root volume). No compile.
- **`compose.quickstart.yaml`** runs one service: SQLite (`mode=rwc`), localfs
  artifacts, loopback bind, and the `AUTH_DISABLED` + `MAIDAN_ALLOW_INSECURE_NO_AUTH`
  dev acknowledgement (required since Cluster 157).
- **`scripts/quickstart-two-agents.sh`** creates a workspace, `planner` + `reviewer`
  agents, a channel and a thread, then posts, reads the shared thread, and replies —
  the collaboration story, not isolated CRUD.
- README gets a "One command (Docker)" section at the top of the quickstart.

## Surprises / decisions

- **Built and ran it, didn't trust the kit.** The external review supplied a starting
  kit but never built the image. Building it surfaced (and fixed) the real issues: the
  runtime base must be `ubuntu:24.04` (the release binary's glibc), and a non-root
  container needs `/data` pre-chowned so a fresh named volume is writable.
- **Verified end-to-end locally:** the image builds (binary SHA verified), `/health`
  reports `v277.0.0` (Clusters 276+277 in the pinned binary), there are **no
  "database is locked"** errors (277's single-connection default is baked into the
  pinned binary — so the quickstart needs none of the review's `MAIDAN_DB_MAX_CONNECTIONS=1`
  workaround), and the two-agent demo posts/reads/replies cleanly.
- **CI guard is deterministic, not flaky.** A full run-the-demo CI job would either pull
  the release binary over the network (flaky, and tests a pinned release rather than the
  PR) or fight the distroless `maidan-server:dev` image's SQLite-volume perms and its
  auth-enabled compose profile. So CI validates the files' integrity (compose `config`
  + `bash -n`) in the compose-smoke job, and the end-to-end is proven locally with a
  full-smoke follow-up logged.

## Capability table extension

| Change | Where |
|--------|-------|
| One-command quickstart (pinned release-binary image) | `docker/Dockerfile.quickstart`, `compose.quickstart.yaml` |
| Two-agent demo script | `scripts/quickstart-two-agents.sh` |
| README quickstart section + CI file-validity guard | `README.md`, `.github/workflows/ci.yml` |

## Risks identified + still open

- **Pinned version drifts.** The Dockerfile pins `v277.0.0` + its SHAs; bump both on a
  new release (documented in the Dockerfile). A future improvement is a published
  `maidan-quickstart` image so users skip even the build.
- **First-admin still uses the dev bootstrap + auth-disabled path.** The
  production-safe `maidan init` is the next backlog item.

## Forward look

Next launch-readiness item: **`maidan init`** — a one-time, production-safe first-admin
bootstrap that seeds the workspace/member/token through the store and refuses on an
initialized database, removing the need for public bootstrap HTTP routes.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 277.0]]. Kit seeded by the external launch-readiness review (Cluster 274),
rebuilt and verified here.
