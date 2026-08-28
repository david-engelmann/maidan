# Cluster 314.0 retro — launch honesty: claims sheet, policies, release verification

> Tag **`v314.0.0`**. Phase XXIV (post-gate hardening). Launch-prep (Launch **L3/L4/L6** +
> Pre-Public Hardening **F2/G5** + A5). No new gate tag.

## What shipped

The launch-prep "honesty" slice — every public claim now maps to evidence, the
contributor policies match the real (solo-maintained) model, and a release is verifiable.

- **Fixed the broken README headline one-liner (L4 discipline earning its keep).** `DATABASE_URL=sqlite::memory: cargo run --bin maidan-server` **did not boot** — auth is on and the server requires a ≥32-byte `MAIDAN_SESSION_SECRET`, so the single most-run command by a stranger errored out before `/health` ever answered. Now it sets a dev signing key; verified it returns `{"status":"ok"}`.
- **L4 — `docs/Claims.md` (published).** An honest claims sheet: every load-bearing README/site claim → a **gate**, a **test/CI job**, or an explicit **"not yet."** Published on the site (SUMMARY + `sync-docs.sh`) and linked from the README. Includes a "not yet / honest limits" section (no hosted SaaS, projectors/email config-gated + unproven in public, SDKs 0.1.0, not on crates.io).
- **L6 — reconciled `CONTRIBUTING.md`** to the honest model: solo-maintained, admin-merge once the **8 required CI checks** pass (dropped the aspirational "one approval required" and the stale "after Cluster A PR #5" reference).
- **L5 / F2 — "Verifying a release" in `SECURITY.md`.** Copy-paste keyless-cosign verify commands for the container image and the release binary/SBOM bundles, with the exact `--certificate-identity-regexp` / `--certificate-oidc-issuer` taken from `release.yml`'s own signing config.
- **L3 / G5 — `CHANGELOG-highlights.md`.** A human digest (what's here since the scale gate) plus a paste-ready GitHub Release-notes template, so tag day isn't a dump of PR titles.

## Surprises / decisions

- **The claims sheet immediately paid for itself** — writing "try it in one line → `{"status":"ok"}`" as a claim forced running it, which exposed the boot failure. That is exactly the L4 loop ("every sentence maps to evidence") working.
- **Published Claims.md links to `SECURITY.md` via a full GitHub URL**, not `../SECURITY.md#…` — `sync-docs.sh`'s root-file rewrite is an exact-paren regex that doesn't match a `#fragment`, and external web links are not fetched by the linkcheck. Cleaner than extending the rewrite.
- **The README "experimental A2A bridge" label now *understates* reality** (full A2A v1.0, arc 282–289). Recorded in the claims sheet as an understatement (safe) rather than silently "fixed" — the README wording is a separate polish call.
- **cosign isn't installed in this environment**, so the verify snippet was validated against `release.yml`'s signing config (identity/issuer) + confirming the `v312.0.0` release actually carries `.cosign.bundle` assets — not by a local `cosign verify` run (a maintainer step).

## Risks / follow-ups

- The claims sheet is hand-maintained; the "How this stays honest" section makes that a
  PR-time obligation. A future doc-lint could assert README claims have a Claims.md row.
- README "experimental A2A" wording could be upgraded to "A2A v1.0" in a later polish pass.

## Launch status

**All launch-prep is now done** (F4 default-secure quickstart in 313; L3/L4/L5/L6 here).
The public launch itself — the public-preview cut, un-hold, and announce — remains
**gated on the maintainer's explicit go** per [[Launch]] (L1–L6). It is not auto-triggered.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes Launch **L3/L4/L6** +
Pre-Public Hardening **F2/G5**; the runbook is [[Launch]], the evidence map is
[Claims](../Claims.md).
