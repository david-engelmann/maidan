# Cluster 124.0 — CI / observability loose ends

**Theme:** Wrap up the follow-ups Clusters 122 and 123 explicitly deferred:
collapse the two overlapping SLO-rule validators into one, and promote the two
new observability CI jobs to required checks.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v124.0.0`**, no new
gate tag.

**Predecessor:** Cluster 122 (`promtool (alert rules)` job + `check-alert-rules.sh`),
Cluster 123 (`otlp smoke` job).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Scripts (124.0.1)** | Delete `scripts/validate-prometheus-rules.sh`; `check-alert-rules.sh` is the sole validator (graceful skip + install hint when promtool is absent). |
| **Docs (124.0.1)** | Repoint `Production.md`, `Capabilities.md`, `docs/alerts/README.md` at the surviving script. |
| **CI (124.0.2)** | Mark `promtool (alert rules)` + `otlp smoke` required in ci.yml comments; `CLAUDE.md` + `Operations.md` → 8 required checks. |
| **Branch protection (124.0.2)** | Add the two contexts to `main`'s required status checks (operator API step, post-merge). |

## Non-goals

- New observability capability — this is consolidation + gating only.
- Path-filtering the heavy jobs (`otlp smoke` runs on every PR, including
  docs-only ones — accepted for simplicity, as with the other smokes).

## PR ladder (actual)

| # | Title |
|---|--------|
| 124.0.1–2 | `ci: consolidate rule-validation scripts + promote alert-rules/otlp to required` (#339) |
| 124.0.retro | `docs(retro): Cluster 124.0 + v124.0.0 tag prep` |

## Exit criteria

- One SLO-rule validator (`check-alert-rules.sh`); no dead `promtool` branch — **met**.
- `main` requires 8 checks incl. `promtool (alert rules)` + `otlp smoke`.
- `v124.0.0` tagged after retro.

## Ordering & risks

- **The deleted script had a dead promtool branch.** It ran `promtool check
  rules` on the `PrometheusRule` CRD (unparseable) behind an uninstalled
  promtool guard — so it never validated. No coverage is lost; metric-name
  presence is still in `alert_templates_contract`.
- **Promotion blocks every subsequent PR on the two jobs.** `otlp smoke` adds
  ~9m even to docs PRs. Accepted (consistent with `scale-out smoke`).
- **Branch-protection PATCH is the operator step**, applied after the impl PR
  merges (the project runs admin-merge anyway).

## References

- [[Retros/Cluster 124.0]], [[Retros/Cluster 122.0]], [[Retros/Cluster 123.0]]
- `scripts/check-alert-rules.sh`, `.github/workflows/ci.yml`, [[Operations]] (branch protection)
