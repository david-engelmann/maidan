### Helm ingress/service gates and release/docs permission pins

- **Fixed:** Helm Ingress refuses empty hosts, non-`Prefix`/`Exact` pathTypes, and a production install without TLS; the Service refuses `NodePort`/`LoadBalancer` when `production` is set.
- **Fixed:** The server Deployment sets `automountServiceAccountToken: false`; a PodDisruptionBudget requires `minAvailable` or `maxUnavailable` when enabled.
- **Fixed:** `release.yml` validates every resolved tag as `vN.N.N`; `docs.yml` scopes `pages:write` / `id-token:write` to the Pages deploy job only.
- **Fixed:** `helm-install-kind-smoke.sh` refuses unsafe image refs and out-of-range local ports.
- **Added:** `scripts/check-helm-ingress-release-gates.sh` locks the contracts in.
