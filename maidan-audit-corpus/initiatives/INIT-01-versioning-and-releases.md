# INIT-01 — Versioning and releases

**Findings:** F-01 (P0, landed), F-02 (P1, values landed / mechanism open), F-03 (P1), F-04 (P2), F-05 (P2)
**Research:** `research/R-03-release-forensics.md`, `research/R-06-quickstart-forensics.md`
**Repo state (2026-09-17):** `main` @ `ca2ddd3`; **tag `v402.0.0` cut** at that commit (D-TAG resolved — see DECISIONS.md). PRs #905 (quickstart pin) and #906 (387 retro) both merged. `CHANGELOG.md` has 400/401/402 entries dated 2026-09-17. `SECURITY.md`'s "latest tagged release" support promise is coherent again. What remains is all mechanism, no longer crisis.

## Problem statement

The acute phase is over: the tag exists, the changelog is current, the evaluation pin is fresh. What remains is the *machinery* that produced the drift, still in place:

1. **Quickstart pins are still hand-maintained (F-02, mechanism half).** #905 moved the values to v349.0.0 and recomputed both SHAs, verified by running. But the four values still live in two files edited by hand: `docker/Dockerfile.quickstart` (`ARG MAIDAN_VERSION`, `ARG MAIDAN_SHA256_AMD64`, `ARG MAIDAN_SHA256_ARM64`) and `compose.quickstart.yaml:24` (`MAIDAN_VERSION: ${MAIDAN_VERSION:-v349.0.0}`). The Dockerfile's own comment begs the human to "bump ALL THREE together … and the default in `compose.quickstart.yaml`, which overrides this one." Note the trap the comment names: **the compose default silently wins over the Dockerfile ARG**, so the Dockerfile's value is dead weight that looks authoritative — #905's retro established that `docker compose config` cannot show this drift. The next release replays the same manual procedure.
2. **README's Docker image still pinned to `:v339.0.0`** (README.md:219, re-verified on `ca2ddd3`) while the release is v402.0.0. Untouched by #905 and by the tag.
3. **`docs/Operations.md` still documents the dead v0.X.0-per-cluster tagging scheme** ("Cluster close"). The repo just demonstrated its actual cadence — batched tag after retro (v402.0.0 covering clusters 400–402) — which contradicts the only written statement of process.
4. **No single machine-readable "latest release" source** (F-05). Four surfaces (tag, CHANGELOG, README pin, compose default) are reconciled today by recency, not by mechanism.

## Why it still matters

The drift class is intact: the next tag will again require a human to remember the Dockerfile ARGs, the compose default, the README pin, and the SHA recompute procedure. #905's body is the evidence that this fails in practice ("the build a stranger runs first was the one with the cross-tenant defects still in it" — v312 predated the Cluster-397 remediation by 88 clusters). The tag fixed the *instance*; nothing fixed the *class*.

## Advisory recommendation (revised)

- **Generate the quickstart pins; don't hand-edit them.** The honest design, accounting for the chicken-and-egg: pins for vN can only be computed *after* vN's tarballs exist, and the tarballs are built by `release.yml` on the tag push. So the flow is: tag → `release.yml` builds tarballs → a follow-up job downloads both per-arch tarballs, computes SHAs, and opens a PR (or pushes a commit) bumping a **single source file** — e.g. `docker/quickstart.versions.env` holding `MAIDAN_VERSION`, `MAIDAN_SHA256_AMD64`, `MAIDAN_SHA256_ARM64`. The Dockerfile takes them as `--build-arg` sourced from that file in CI; the compose file reads the same file (`env_file` or variable interpolation) so there is exactly one default, not two competing ones. A design that pretends `release.yml` can pin vN *during* the vN build misunderstands the pipeline; the follow-up PR is the correct shape, and it should say so in a comment.
- **Fold the README image pin into the same mechanism** (or at minimum bump it to v402.0.0 now — it's 63 clusters stale and it's the first Docker reference a reader hits).
- **Write down the actual cadence.** `docs/Operations.md`'s tagging section should describe what just happened (retro → batched tag → `release.yml`), marked as the current practice, with the v0.X.0-per-cluster text moved to history. An undocumented cadence will drift again; a wrongly-documented one already did.
- **Keep #905's verification method as policy:** pin changes are verified by booting the stack and reading `/health`, not by reading the file. Automate it: a CI job that boots `compose.quickstart.yaml` and asserts `/health`'s version matches the expected pin.
- Alternatives considered: a `:latest`-style floating tag removes staleness but destroys reproducibility (rejected before, still rejected); docs-only reminders preserve the failing mechanism (rejected).

## Open questions for the building agent

- Should the quickstart track the latest tag, or latest *stable* tag? #905 pinned to the newest tag that existed; that was availability, not policy. (If a "stable" concept ever exists, the versions file is where it would live.)
- Does anything downstream (Helm chart, docs site) assume the current ad-hoc pin locations? The Helm `values-prod.yaml` empty `image:` block (F-31) suggests the chart doesn't pin at all — confirm before moving files.
- Who owns the follow-up pin PR — automation with maintainer merge, or fully automatic commit? (Fully automatic version bumps on `main` interact with the retro-before-tag rule; think it through.)

## Signals of resolution

- `docker/Dockerfile.quickstart` and `compose.quickstart.yaml` contain no hand-maintained version or SHA; one generated file holds all four values.
- A CI job boots the quickstart stack and asserts the `/health` version matches the pin.
- README's Docker pin, CHANGELOG, tag, and compose default agree by mechanism, not by recency.
- `docs/Operations.md` describes the actual tagging practice.
