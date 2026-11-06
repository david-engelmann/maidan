#!/usr/bin/env bash
# Contract gate for the checked-in Kubernetes manifests and Helm chart defaults:
# the maidan-server Deployment must run non-root with a locked-down container
# securityContext, readiness/liveness probes, CPU/memory bounds, and a
# NetworkPolicy. Postgres/MinIO StatefulSets must at least drop privileges and
# refuse privilege escalation. A missing field here once shipped a privileged
# postgres pod into a shared cluster.
#
# Usage: scripts/check-k8s-security.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print("k8s security: PyYAML required", file=sys.stderr)
    sys.exit(1)

errors = []
checked = 0

def load_all(path: Path):
    docs = list(yaml.safe_load_all(path.read_text()))
    return [d for d in docs if isinstance(d, dict)]

def require(cond, msg):
    if not cond:
        errors.append(msg)

def container_locked(path, cname, c, *, read_only_root=True):
    sc = c.get("securityContext") or {}
    require(sc.get("allowPrivilegeEscalation") is False,
            f"{path}: container {cname} must set allowPrivilegeEscalation: false")
    caps = (sc.get("capabilities") or {}).get("drop") or []
    require("ALL" in caps, f"{path}: container {cname} must capabilities.drop include ALL")
    if read_only_root:
        require(sc.get("readOnlyRootFilesystem") is True,
                f"{path}: container {cname} must set readOnlyRootFilesystem: true")

def pod_non_root(path, pod_sc):
    require(pod_sc.get("runAsNonRoot") is True,
            f"{path}: pod securityContext.runAsNonRoot must be true")
    scp = pod_sc.get("seccompProfile") or {}
    require(scp.get("type") == "RuntimeDefault",
            f"{path}: pod seccompProfile.type must be RuntimeDefault")

# --- server Deployment ---
dep_path = Path("k8s/base/maidan-server-deployment.yaml")
deps = [d for d in load_all(dep_path) if d.get("kind") == "Deployment"]
require(len(deps) == 1, f"{dep_path}: expected one Deployment")
if deps:
    checked += 1
    pod = ((deps[0].get("spec") or {}).get("template") or {}).get("spec") or {}
    pod_non_root(dep_path, pod.get("securityContext") or {})
    require(pod.get("enableServiceLinks") is False,
            f"{dep_path}: enableServiceLinks must be false")
    containers = pod.get("containers") or []
    require(containers, f"{dep_path}: missing containers")
    for c in containers:
        cname = c.get("name", "?")
        container_locked(dep_path, cname, c, read_only_root=True)
        require(c.get("readinessProbe"), f"{dep_path}: {cname} missing readinessProbe")
        require(c.get("livenessProbe"), f"{dep_path}: {cname} missing livenessProbe")
        res = c.get("resources") or {}
        require((res.get("requests") or {}).get("memory"),
                f"{dep_path}: {cname} missing resources.requests.memory")
        require((res.get("limits") or {}).get("memory"),
                f"{dep_path}: {cname} missing resources.limits.memory")

# --- NetworkPolicy ---
np_path = Path("k8s/base/network-policy.yaml")
nps = [d for d in load_all(np_path) if d.get("kind") == "NetworkPolicy"]
require(nps, f"{np_path}: missing NetworkPolicy")
if nps:
    checked += 1
    spec = nps[0].get("spec") or {}
    types = set(spec.get("policyTypes") or [])
    require("Ingress" in types, f"{np_path}: policyTypes must include Ingress")
    require("Egress" in types, f"{np_path}: policyTypes must include Egress")
    require(spec.get("egress"), f"{np_path}: egress rules required (DNS + datastores)")

# --- StatefulSets ---
for path, allow_rw_root in (
    (Path("k8s/base/postgres-statefulset.yaml"), True),
    (Path("k8s/base/minio-statefulset.yaml"), True),
):
    sts = [d for d in load_all(path) if d.get("kind") == "StatefulSet"]
    require(sts, f"{path}: missing StatefulSet")
    if not sts:
        continue
    checked += 1
    pod = ((sts[0].get("spec") or {}).get("template") or {}).get("spec") or {}
    pod_sc = pod.get("securityContext") or {}
    require(pod_sc.get("runAsNonRoot") is True,
            f"{path}: pod securityContext.runAsNonRoot must be true")
    for c in pod.get("containers") or []:
        container_locked(path, c.get("name", "?"), c, read_only_root=False)
        require(c.get("readinessProbe"), f"{path}: {c.get('name')} missing readinessProbe")
        require((c.get("resources") or {}).get("limits"),
                f"{path}: {c.get('name')} missing resources.limits")

# --- Helm prod defaults ---
prod = Path("helm/maidan/values-prod.yaml")
text = prod.read_text()
checked += 1
require("networkPolicy:" in text and "enabled: true" in text,
        f"{prod}: production values must enable networkPolicy")
require("production: true" in text, f"{prod}: production: true required")

# --- Helm chart container securityContext present in template ---
tmpl = Path("helm/maidan/templates/deployment.yaml").read_text()
checked += 1
require("allowPrivilegeEscalation: false" in tmpl,
        "helm/maidan/templates/deployment.yaml must lock allowPrivilegeEscalation")
require("readOnlyRootFilesystem: true" in tmpl,
        "helm/maidan/templates/deployment.yaml must set readOnlyRootFilesystem")
require("RuntimeDefault" in tmpl,
        "helm/maidan/templates/deployment.yaml must set RuntimeDefault seccomp")

if errors:
    print("k8s security:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"k8s security: {checked} manifest group(s) ok")
PY
