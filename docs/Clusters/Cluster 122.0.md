# Cluster 122.0 — Execute the alert rules

**Theme:** Stop trusting the SLO alert rules by inspection — execute them in
CI. Close the gap Cluster 121's retro named ("alert exprs aren't executed in
CI; only metric names are contract-checked"), and correct the OTLP-status
misstatement 121 introduced.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v122.0.0`**, no new
gate tag.

**Predecessor:** Cluster 90 (SLO rules + dashboard), Cluster 121 (scale-out
SLO coverage + the `$value` bug this cluster fixes), Cluster 89 (OTLP export —
the subject of the doc correction).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **CI (122.0.1)** | `promtool (alert rules)` job — `promtool check rules` + `promtool test rules` over the SLO rules. |
| **Tooling (122.0.1)** | `scripts/check-alert-rules.sh` — extract `.spec` from the PrometheusRule CRD into a raw rules file, then run promtool. |
| **Tests (122.0.1)** | `prometheus-rules-maidan-slo.test.yaml` — unit tests pinning the queue-saturation guard, embed-failure restart-safety, and `$value` rendering. |
| **Fix (122.0.1)** | `MaidanIndexerQueueSaturated` expr reorder so `$value` is the fill fraction, not capacity (was rendering "1000% full"). |
| **Docs (122.0.2)** | Correct the OTLP-export status (shipped in 89, not open) in Remaining Work §1/§3, CHANGELOG, and the 121 plan/retro. |

## Non-goals

- Promoting `promtool (alert rules)` to a required check (left as a follow-up;
  it runs on every PR).
- OTLP end-to-end collector smoke (the real residual observability gap).
- Alertmanager routing/inhibition tests (rules only).

## PR ladder (actual)

| # | Title |
|---|--------|
| 122.0.1–2 | `feat(ci): execute SLO alert rules with promtool + fix queue-sat annotation` (#335) |
| 122.0.retro | `docs(retro): Cluster 122.0 + v122.0.0 tag prep` |

## Exit criteria

- `promtool check rules` + `promtool test rules` run in CI on every PR — **met**.
- The unit tests assert the queue-saturation guard and embed-failure
  restart-safety (the Cluster 121 semantics) — **met**.
- OTLP-export status corrected to "shipped in 89" everywhere — **met**.
- `v122.0.0` tagged after retro.

## Ordering & risks

- **CRD ≠ raw rules format.** `promtool check rules` wants a top-level
  `groups:` document; our rules are a `PrometheusRule` CRD. The script extracts
  `.spec` first (yq on CI, python3+PyYAML locally).
- **Annotation assertions are exact.** promtool compares rendered annotations
  byte-for-byte; the fixture reproduces the rules' line breaks. This is
  deliberate — it locks in the `$value` rendering that the bug got wrong.
- **promtool pinned to 3.12.0** in CI (downloaded from the Prometheus release).

## References

- [[Retros/Cluster 122.0]], [[Retros/Cluster 121.0]], [[Retros/Cluster 90.0]], [[Retros/Cluster 89.0]]
- `scripts/check-alert-rules.sh`, `docs/alerts/prometheus-rules-maidan-slo.test.yaml`
- [[Remaining Work]] §1/§3 (OTLP correction)
