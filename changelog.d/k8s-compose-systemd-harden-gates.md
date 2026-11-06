### Kubernetes securityContext and NetworkPolicy hardening

- **Added:** `scripts/check-k8s-security.sh` requires the server Deployment's non-root/seccomp/probe/resource locks, StatefulSet privilege drops, and an Ingress+Egress NetworkPolicy.
- **Fixed:** Postgres and MinIO StatefulSets run as non-root with `allowPrivilegeEscalation: false` and `capabilities.drop: [ALL]`.
- **Fixed:** `k8s/base/network-policy.yaml` now denies egress except DNS and in-namespace Postgres/MinIO.
- **Changed:** `helm/maidan/values-prod.yaml` enables NetworkPolicy with ingress-nginx and datastore egress defaults.
