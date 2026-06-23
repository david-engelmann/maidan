# Cluster 124.0 retro — CI / observability loose ends

> Tag **`v124.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **One SLO-rule validator** (124.0.1): deleted `scripts/validate-prometheus-rules.sh`
  and made `scripts/check-alert-rules.sh` the sole entry point, with a graceful
  skip + install hint when `promtool` is absent (absorbing the old script's
  local behavior). Repointed `Production.md`, `Capabilities.md`, and
  `docs/alerts/README.md`.
- **Two jobs promoted to required** (124.0.2): `promtool (alert rules)` and
  `otlp smoke` added to `main`'s branch-protection required checks (now **8**),
  with ci.yml comments + `CLAUDE.md`/`Operations.md` updated to match.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| — | — | Nothing carried; this cluster *was* the deferral list from 122/123. |

## Surprises

- **The old validator never validated.** `validate-prometheus-rules.sh` had a
  `promtool check rules` call — but it ran on the `PrometheusRule` CRD (which
  promtool can't parse) *and* was guarded behind `command -v promtool`, which
  was never installed in CI before Cluster 122. So the "validation script" only
  ever did substring greps (already covered by `alert_templates_contract`).
  Deleting it loses nothing and removes a latent foot-gun (it would have errored
  the moment someone installed promtool and pointed it at the CRD).

## Decisions

- **Consolidate to the CI-wired script.** `check-alert-rules.sh` is what CI
  runs and what actually works (CRD extraction + unit tests); the older script
  was a strictly-weaker duplicate.
- **Graceful skip, not hard fail, when promtool is missing.** Local `cargo`
  workflows shouldn't be blocked by a missing external tool; CI (promtool
  installed, required job) is the real gate.
- **Promote both new safety nets.** A check that doesn't block merges is a
  suggestion; 122/123's value lands only once `promtool (alert rules)` and
  `otlp smoke` are required.

## Capability table extension

| Capability | Where |
|------------|-------|
| Single SLO-rule validator (promtool check + unit tests) | `scripts/check-alert-rules.sh` |
| 8 required status checks (incl. alert-rules + otlp smoke) | branch protection on `main`; [[Operations]] |

## Risks identified + still open

- **Heavy required jobs on every PR.** `otlp smoke` (~9m) and `scale-out smoke`
  (~9m) now gate even docs-only PRs. Path-filtering could trim this but adds its
  own failure mode (a skipped required check blocks merge); left as-is.

## Forward look

The observability arc (export → SLO rules → promtool → collector smoke) is
complete and gated. The largest remaining named gap is event-bus delivery
dedup / idempotency (the oldest standing risk); a backlog truth-pass is the
lower-effort alternative.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
