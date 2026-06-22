# Cluster 122.0 retro — Execute the alert rules

> Tag **`v122.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`promtool (alert rules)` CI job** (122.0.1): runs `promtool check rules`
  (lint PromQL + Go templates) and `promtool test rules` (unit tests) over the
  SLO rules on every PR. promtool 3.12.0 is fetched from the Prometheus release.
- **`scripts/check-alert-rules.sh`** (122.0.1): the rules ship as a Kubernetes
  `PrometheusRule` CRD, but promtool wants a raw `groups:` document — the script
  extracts `.spec` into a git-ignored `slo-rules.generated.yaml` (yq on CI,
  python3+PyYAML locally), then runs promtool from `docs/alerts/`.
- **`prometheus-rules-maidan-slo.test.yaml`** (122.0.1): unit tests that pin the
  Cluster 121 semantics — `MaidanIndexerQueueSaturated` fires >80% full and is
  guarded off at capacity 0; `MaidanIndexerEmbedFailures` fires on a rising
  delta but NOT on a reset-to-0 (restart-safe). Plus the recording-rule ratio
  and a pre-existing outbox alert as sanity.
- **A real bug fixed** (122.0.1): see Surprises.
- **OTLP-status doc correction** (122.0.2): OTLP export (traces + metrics
  fanout) shipped in Cluster 89 — it was never the open deferral the 121 docs
  claimed. Corrected in Remaining Work §1/§3, CHANGELOG, and the 121 plan/retro.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Follow-up | Promote `promtool (alert rules)` to a required check | Runs on every PR; promotion is a branch-protection op. |
| Cluster 89+ | OTLP end-to-end collector smoke | The real residual observability gap; unit-tested + documented today, not asserted against a running collector. |
| Out of scope | Alertmanager routing/inhibition tests | This cluster validates rules, not routing. |

## Surprises

- **Executing the rules found a bug on the first run.** `MaidanIndexerQueueSaturated`
  used `maidan_indexer_queue_capacity > 0 and maidan_slo:indexer_queue_saturation > 0.8`.
  In PromQL `a and b` returns the **LHS** value, so the alert's `$value` was the
  *capacity* (e.g. 10) and `{{ $value | humanizePercentage }}` rendered
  **"1000% full"**. Swapping to `saturation > 0.8 and capacity > 0` makes
  `$value` the fill fraction (**"90% full"**) and keeps the capacity guard. The
  name-only `alert_templates_contract` could never have caught this.
- **promtool compares annotations exactly.** Omitting `exp_annotations` means
  "expect none", so the test fails against an alert that *has* annotations. The
  fixture reproduces the rules' exact line breaks — turning a brittleness into a
  feature: it locks in the corrected `$value` rendering.
- **The OTLP "gap" was a phantom.** Chasing the next gap, the backlog said OTLP
  export was open; the code said otherwise (`build_otlp_metrics_recorder` wired
  into `metrics::init`, traces in `observability::init`, documented, tested).
  The 121 docs had propagated the stale claim — and I'd reinforced it. Reading
  the code, not the backlog, is the lesson.

## Decisions

- **Execute, don't inspect.** A contract that checks metric *names* proves the
  rules mention the right series; only running the PromQL proves they *behave*.
  The bug is the proof.
- **Extract from the CRD, keep one source of truth.** The CRD stays canonical
  (it's what ships to the cluster); the raw rules file is generated and
  git-ignored, never committed — no drift.
- **Correct the record in place + note it.** The OTLP misstatement is fixed in
  the forward-looking docs and annotated (not silently rewritten) in the 121
  retro/plan, so the correction is visible.

## Capability table extension

| Capability | Where |
|------------|-------|
| SLO alert/recording PromQL executed in CI (`check` + unit tests) | `.github/workflows/ci.yml` (`promtool (alert rules)`), `scripts/check-alert-rules.sh` |
| SLO rule unit tests (guard + restart-safety + `$value`) | `docs/alerts/prometheus-rules-maidan-slo.test.yaml` |

## Risks identified + still open

- **promtool job is non-required.** It runs on every PR but doesn't block until
  promoted in branch protection (documented as a follow-up).
- **OTLP delivery still unproven end-to-end in CI.** Unit-tested + documented;
  no collector-delivery assertion (the residual gap).

## Forward look

Post-gate hardening continues from [[Remaining Work]] / [[Open Work]].
Candidate next: OTLP end-to-end collector smoke, durable async reindex job, or
event-bus delivery dedup (the oldest standing risk).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
