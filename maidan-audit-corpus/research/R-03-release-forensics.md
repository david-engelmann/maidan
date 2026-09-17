# R-03 — Release forensics

## The numbers

| Source | Value at audit time | Value at third pass (2026-09-16 eve) |
|--------|---------------------|----------------------------------------|
| Latest git tag | `v349.0.0` (2026-09-01) | `v349.0.0` (2026-09-01) — still newest release |
| `CHANGELOG.md` head | 398.0.0 | 398.0.0 |
| `main` HEAD | Cluster 399.2 (`e232c73`) | Cluster 400.5 (`dab377a`) |
| Gap, tag → HEAD | ~285 PRs | **261 commits** (per #905) |
| README Docker pin | `:v339.0.0` | `:v339.0.0` (README.md:219, re-verified) |
| Quickstart binary pin | v312.0.0 | v312.0.0 on `main`; **#905 (in CI) → v349.0.0** |
| `docs/Operations.md` tagging scheme | v0.X.0 per cluster ("Cluster close") — not followed | unchanged |

## What #905 established (read 2026-09-16, in CI at corpus time)

- **The stale pin was a security defect, not a freshness nit.** v312.0.0 predates the entire Cluster-397 authorization remediation: "the build a stranger runs first was the one with the cross-tenant defects still in it." The documented one-command path served a binary 37 releases old.
- **Found by running, not reading.** The stack reported `"version":"v312.0.0"` on `/health` on a machine whose source was at Cluster 400 — the pin "looked fine in the file."
- **The drift mechanism, precisely:** two places held the same pin and only one was obvious — the `compose.quickstart.yaml` default silently wins over the `Dockerfile.quickstart` ARG, so `docker compose config` would not have shown the drift either.
- **What #905 changes:** both pins (Dockerfile ARG and compose default) plus both per-arch SHA-256s, recomputed from the published v349.0.0 tarballs; comment now says all three values move together and gives the recompute command. Verified end to end: image builds, hash check passes, boots, reports `"version":"v349.0.0"`.
- **What #905 deliberately does not do:** cut a tag ("a tag triggers `release.yml`" — maintainer action, flagged not done), or automate the pins (mechanism still hand-maintained).
- **The tag gap's concrete consumer cost:** `GET /threads/:id/deliveries` landed in Cluster 379 — confirmed by running v349.0.0 against the soundcheck integration's exact call sequence (201/201/404-documented/404-route-missing). It exists in no published release; consumers must build from `main`.

## How the gap was established

Tags were listed from the repo; `CHANGELOG.md`'s top entry was read; HEAD's cluster pointer was taken from the repo's own cluster tracking. The three were compared directly — no inference required. The ~285-PR figure comes from the commit distance between `v349.0.0` and HEAD.

## Why "stalled" and not "deliberate"

Three pieces of evidence point to automation breaking rather than a deliberate cadence change:

1. `CHANGELOG.md` kept advancing to 398.0.0 — the *intent* to version per cluster survived; only the tags stopped.
2. `docs/Operations.md` still documents the per-cluster tagging process as current — nobody updated the process docs, which is what you'd expect if the cadence had been deliberately changed.
3. `SECURITY.md`'s support promise ("latest minor release") was left pointing at a tag 261 commits old — a deliberate change would have updated the promise.

**Third-pass update:** #905 (2026-09-16) treats the missing tag as overdue maintainer action — "flagged here rather than done" — not as a deliberate cadence. The counter-hypothesis (silent move to tag-on-release-branch) is now weaker than at audit time, but still unchecked: if a new cadence was chosen, it remains unwritten.

## The pin-staleness mechanism

`compose.quickstart.yaml` and `docker/Dockerfile.quickstart` carry version numbers and SHA checksums edited by hand — verified at 399.3: `docker/Dockerfile.quickstart:13` (`ARG MAIDAN_VERSION=v312.0.0`), lines 16–17 (per-arch SHA-256 ARGs with a "Bump both when MAIDAN_VERSION changes" comment), and `compose.quickstart.yaml:24` (`MAIDAN_VERSION: ${MAIDAN_VERSION:-v312.0.0}`). The audit's claim is structural, not incidental: **any release process that requires a human to remember N files will produce exactly the observed drift** (v312.0.0 pin vs Cluster 399.2 HEAD = ~87 clusters of drift). The README's `:v339.0.0` pin (README.md:219, verified) is the same mechanism. This is why INIT-01 recommends generated pins rather than "remember to update the pins."
