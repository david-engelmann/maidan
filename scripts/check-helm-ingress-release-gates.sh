#!/usr/bin/env bash
# Contract gate for Helm ingress/service/deployment/PDB hardening, release
# tag shape, docs Pages permission scope, and kind-smoke port/image bounds.
#
# Usage: scripts/check-helm-ingress-release-gates.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import re
import sys
from pathlib import Path

errors = []
checked = 0

def require(cond, msg):
    if not cond:
        errors.append(msg)

ing = Path("helm/maidan/templates/ingress.yaml").read_text()
checked += 1
require("production ingress needs tls" in ing,
        "ingress.yaml must require TLS in production")
require("pathType %q must be Prefix or Exact" in ing or 'must be Prefix or Exact' in ing,
        "ingress.yaml must require pathType Prefix or Exact")
require("ingress.hosts is empty" in ing,
        "ingress.yaml must refuse an empty hosts list")

dep = Path("helm/maidan/templates/deployment.yaml").read_text()
checked += 1
require(re.search(r"(?m)^\s*automountServiceAccountToken:\s*false\s*$", dep),
        "deployment.yaml must set automountServiceAccountToken: false")

svc = Path("helm/maidan/templates/service.yaml").read_text()
checked += 1
require("not allowed in production" in svc and "ClusterIP" in svc,
        "service.yaml must refuse NodePort/LoadBalancer in production")

pdb = Path("helm/maidan/templates/poddisruptionbudget.yaml").read_text()
checked += 1
require("minAvailable or maxUnavailable" in pdb,
        "poddisruptionbudget.yaml must require a budget when enabled")

rel = Path(".github/workflows/release.yml").read_text()
checked += 1
require(rel.count("not a release tag") >= 5,
        "release.yml must validate vN.N.N on tag resolve steps")
require(rel.count('TAG is empty') >= 5,
        "release.yml must refuse an empty TAG")

docs = Path(".github/workflows/docs.yml").read_text()
checked += 1
# Top-level must not grant pages write
head = docs.split("jobs:", 1)[0]
require("pages: write" not in head,
        "docs.yml must not grant pages: write at workflow scope")
require("id-token: write" not in head,
        "docs.yml must not grant id-token: write at workflow scope")
require("pages: write" in docs and "id-token: write" in docs,
        "docs.yml deploy job must still request pages + id-token write")

smoke = Path("scripts/helm-install-kind-smoke.sh").read_text()
checked += 1
require("refusing image ref" in smoke,
        "helm-install-kind-smoke.sh must refuse unsafe image refs")
require("port out of range" in smoke,
        "helm-install-kind-smoke.sh must bound local ports")

if errors:
    print("helm-ingress-release gates:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"helm-ingress-release gates: {checked} check(s) ok")
PY
