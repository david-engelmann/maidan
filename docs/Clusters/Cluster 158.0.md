# Cluster 158.0 — signed container images (keyless cosign)

**Theme:** Enterprise-hardening arc (arc 1), part 3. Sign the container images so
they're verifiable by an admission controller — release blobs were signed, the
images weren't.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v158.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `sign-images` job: keyless `cosign sign` of the server + postgres images, by digest | `.github/workflows/release.yml` |
| Verification instructions | `docs/Operations.md` |

## Why

Track V.3 keyless-signed the release **blobs** (tarballs + SBOM), but the
container images pushed to ghcr had no signature — the production-readiness
review's supply-chain gap. Without an image signature, a Kyverno / Sigstore
admission policy cannot verify what it's admitting, and a registry-tamper or
tag-repoint is undetectable.

## Non-goals

- **trivy (or grype) image vuln scanning** — deferred to the arc-2 CI cluster,
  where it fits thematically and the scanner action can be pinned properly
  (release.yml is not exercised by PR CI, so an unpinned scanner step would be
  un-pre-testable in a supply-chain cluster).
- SBOM-as-image-attestation (`cosign attest`) — a possible follow-up.

## PR ladder (actual)

| # | Title |
|---|--------|
| 158.0.1 | `ci(release): cosign-sign container images by digest` (#406) |
| 158.0.retro | `docs(retro): Cluster 158.0 + v158.0.0 tag prep` |

## Exit criteria

- Both images are cosign-signed by digest on the release run; verification is
  documented — **met on impl**; the signatures themselves are produced on the
  `v158.0.0` tag run (release.yml is tag-triggered).
- `v158.0.0` tagged after retro.

## Verification & limits

- The YAML parses and the new job reuses only actions already present in this
  workflow (`setup-buildx`, `login-action`, `cosign-installer`). **PR CI does not
  run `release.yml`** — the job first executes on the `v158.0.0` tag; check that
  release run for the signatures and `cosign verify` per `Operations.md`.
- Signs by the immutable index digest (`imagetools inspect --format
  '{{.Manifest.Digest}}'`), not the mutable tag, so a re-point can't evade it.

## References

- [[Retros/Cluster 158.0]]; `release.yml` (`sign-images`), [[Operations]]
  (verify command). Program: [[Roadmap]] + memory `maidan-next-arc-program`.
