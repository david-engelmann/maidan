# Cluster 185.0 — SaaS ops: Helm hardening (probes, PDB, NetworkPolicy, existingSecret)

**Theme:** Arc B (multi-tenant SaaS operability), part 1 — the operational
quick-wins that make the chart safe to run at scale on a real cluster.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v185.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Liveness/startup → shallow `/health/live`; readiness → deep `/health/ready` | `templates/deployment.yaml`, `values.yaml` (`probes`) |
| `PodDisruptionBudget` (opt-in; on in prod) | `templates/poddisruptionbudget.yaml`, `values.yaml`, `values-prod.yaml` |
| `NetworkPolicy` (opt-in, safe-by-default egress) | `templates/networkpolicy.yaml`, `values.yaml` |
| `existingSecret` — reference a pre-created Secret; chart renders none | `templates/secret.yaml`, `_helpers.tpl` (`maidan.secretName`), `values.yaml` |

## Why

- **Restart-storm bug.** Both probes hit `/health`, which returns **`503` when any
  dependency is degraded** (`readiness()` checks db/storage/indexer/bus). So a
  transient DB blip failed the *liveness* probe → k8s killed the pod → it
  restarted while the DB was still recovering, amplifying an outage. Liveness
  must be shallow. The always-`200` `/health/live` endpoint already existed but
  went unused by the chart.
- **No disruption budget / network policy / external-secret support** — table
  stakes for running a tenant workload on a shared cluster.

## The fix

- **Probes**: `startupProbe` + `livenessProbe` → `/health/live` (process alive;
  a degraded dependency no longer restarts the pod — it drains from the Service
  instead and recovers when the dependency does); `readinessProbe` →
  `/health/ready` (the same deep check as before, now on its explicit endpoint).
  The startup probe (30 × 5 s = ~150 s grace) protects slow starts/migrations.
- **PodDisruptionBudget**: opt-in template; enabled with `minAvailable: 1` in
  `values-prod.yaml` (which runs ≥ 2 replicas) so node drains/rollouts keep a pod
  serving. `maxUnavailable` wins over `minAvailable` when set.
- **NetworkPolicy**: opt-in, safe-by-default — ingress restricted to the HTTP
  port (source-configurable via `ingressFrom`), egress open by default
  (`allowAllEgress: true`, so datastores work) with DNS always allowed; operators
  set `allowAllEgress: false` + `egress` to lock it down.
- **existingSecret**: `maidan.secretName` resolves to `.Values.existingSecret`
  when set (and `secret.yaml` renders nothing), else the chart-managed Secret —
  keeping secret material out of values files / release history.

## Exit criteria

- Liveness is shallow (no restart storm on a DB blip); PDB/NetworkPolicy/
  existingSecret are available and render on every values combo — **met**.
- `v185.0.0` tagged.

## Verification & limits

- `helm lint` (default/CI/prod) clean; `scripts/helm-template-smoke.sh` (all
  values combos) OK; rendered every branch (existingSecret suppresses the chart
  Secret and repoints `secretRef`; PDB `minAvailable`/`maxUnavailable`;
  NetworkPolicy `allowAllEgress` on/off). The `helm install (kind)` smoke is
  unaffected — readiness stays on the same handler (CI uses SQLite, so it passes
  without Postgres) and the startup probe on `/health/live` succeeds immediately.
- Limits: NetworkPolicy default leaves egress open (correctness over lockdown —
  a too-strict default silently breaks DB connectivity); tightening is a
  documented per-cluster step. PDB is opt-in (a single-replica dev install with a
  PDB could block node drains).

## References

- [[Retros/Cluster 185.0]]; `helm/maidan/`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Arc B).
