# Track X — Release engineering

Cross-cutting track (no version tag). Automates and hardens the path from
retro merge to installable artifacts.

> **Goal:** Reliable GitHub Releases for every tag, pinned prod images,
> SBOM optional.
>
> **Not a cluster** — ship as numbered PRs on `main`.

## PRs (proposed)

| #    | Title | Source |
|------|-------|--------|
| X.1  | `ci: verify release.yml on tag push (smoke)` | Open Work |
| X.2  | `chore(k8s): pin image digests in prod overlay` | k8s README |
| X.3  | `chore: cargo-cyclonedx SBOM in release job` | Cluster B retro |

## Exit criteria

- Documented checklist: tag → Release assets → ghcr images.
- Prod overlay uses digests, not floating `:latest`.

## Out of scope

- Changing semver policy (see [[Decisions]] / Cluster 1.0).
