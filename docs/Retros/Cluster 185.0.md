# Cluster 185.0 retro — the chart stops restart-storming on a DB blip

> Tag **`v185.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc B (multi-tenant SaaS operability), part 1.

## What shipped

- Liveness + startup probes moved to the shallow `/health/live`; readiness to the
  deep `/health/ready`. Opt-in `PodDisruptionBudget` (on in prod) and
  `NetworkPolicy` templates. `existingSecret` support via a `maidan.secretName`
  helper + conditional `secret.yaml`.

## Surprises

- **The dangerous probe was liveness, and the fix already existed.** `/health`
  returns `503` on any degraded dependency, and *liveness* was pointed at it — so
  a transient DB blip literally instructed Kubernetes to kill the pod mid-outage.
  The correct shallow endpoint (`/health/live`, always `200`) had shipped long
  ago but the chart never used it. A one-line path change removes a genuine
  outage-amplifier.
- **A startupProbe reshapes the other probes for free.** Once a startupProbe is
  present, k8s suspends liveness *and* readiness until it passes — so putting it
  on the always-`200` `/health/live` gives slow starts/migrations up to ~150 s of
  grace without touching the deep readiness gate, and it succeeds immediately in
  the CI smoke so `helm install --wait` is unaffected.

## Decisions

- **NetworkPolicy: correctness over lockdown by default.** A restrictive egress
  default silently breaks datastore connectivity — the worst kind of chart bug
  (renders fine, fails at runtime). So the default allows egress (DNS always
  open) and only restricts ingress to the HTTP port; tightening egress is an
  explicit, documented per-cluster step. Opt-in overall (`enabled: false`).
- **PDB opt-in, on in prod.** A PDB on a single-replica dev install would block
  node drains; `values-prod.yaml` (≥ 2 replicas) enables `minAvailable: 1`.
- **existingSecret over sealed-secret plumbing.** The lightest thing that keeps
  secrets out of values/history — reference a Secret the operator already manages
  (Vault, SOPS, sealed-secrets, whatever). No opinion imposed.

## Capability table extension

| Change | Where |
|--------|-------|
| Shallow-liveness probes + PDB + NetworkPolicy + existingSecret | `helm/maidan/` |

## Risks identified + still open

- **Net risk-reducing, no default behaviour change to a running install beyond
  the (safer) probe endpoints.** All new features are opt-in (PDB/NetworkPolicy/
  existingSecret) except the probe-path fix, which is strictly more lenient on
  liveness and identical on readiness. Verified across every values combo.

## Forward look

Arc B continues: workspace export/portability, data-retention pruning
(events/audit/deliveries grow unbounded), per-tenant metrics/metering, and a
secret-rotation keyring.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
