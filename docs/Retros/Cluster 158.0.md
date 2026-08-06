# Cluster 158.0 retro — signed container images

> Tag **`v158.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Enterprise-hardening arc (arc 1), part 3 — concludes the hardening quick-wins.

## What shipped

- **`sign-images` release job** — after the server manifest + postgres push, it
  resolves each tag to its immutable index digest and keyless-`cosign sign`s the
  digest for `maidan-server` and `maidan-postgres`, using the workflow's GitHub
  OIDC identity (no private key — same trust root as the blob signatures).
- **Verification documented** in `Operations.md` (`cosign verify` with the OIDC
  issuer + identity regexp; note it's admission-controller-ready).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Arc 2 (CI/CD) | trivy image vuln scan | Fits the CI arc; lets the scanner action be pinned. `release.yml` isn't PR-tested, so an unpinned scanner would be un-pre-testable in the very cluster meant to *harden* the supply chain. |
| Future | `cosign attest` SBOM-on-image | Nice-to-have provenance; not required to close the "unsigned images" gap. |

## Surprises

- **`release.yml` is a blind spot in CI.** None of the 8 required checks (or the
  advisory ones) exercise the release workflow — it only runs on a `v*` tag. So
  this cluster's real validation happens on the `v158.0.0` tag push, not in the
  impl PR. That shaped the scope (defer anything I can't author with high
  confidence) and the retro note to check the release run.

## Decisions

- **Sign the digest, not the tag.** A signature bound to a mutable tag could be
  evaded by re-pointing the tag; the index digest is the artifact's true
  identity, and `cosign verify <tag>` still resolves to it.
- **Reuse the existing keyless OIDC identity** so image and blob verification
  share one `--certificate-identity-regexp` / issuer — a single trust root for
  everything the release publishes.

## Capability table extension

| Capability | Where |
|------------|-------|
| Signed container images (keyless cosign, by digest) | `release.yml` `sign-images` |

## Risks identified + still open

- **Low, but tag-only validated.** A misconfig would fail the `sign-images` job
  on the release run (visible), not corrupt artifacts — the images are pushed by
  earlier jobs. Confirm the first signed release (`v158.0.0`) and the
  `cosign verify` output.

## Forward look — arc 1 pivots to its flagship

The hardening quick-wins (156 shutdown/timeout, 157 fail-closed auth, 158 signed
images) are done. Arc 1 now turns to the **flagship: channel/thread RBAC** — the
#1 research finding (authz is workspace-flat). Planned as three clusters —
membership model (additive, zero blast radius) → enforcement (public/private
semantics, `__dm__` exempt) → management API + `channel:admin`; Postgres RLS
deferred. Then arcs 2 (perf + CI/CD, incl. the deferred trivy scan), 3 (agentic
features), 4 (token round 3).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
