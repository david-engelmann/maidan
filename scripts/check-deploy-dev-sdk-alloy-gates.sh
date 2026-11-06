#!/usr/bin/env bash
# Contract gate for deploy-dev bootstrap pin, sdk-release tag shape, Alloy
# attribute redaction, clean-machine release tags, and Helm NetworkPolicy
# fail-closed egress.
#
# Usage: scripts/check-deploy-dev-sdk-alloy-gates.sh
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

dev = Path(".github/workflows/deploy-dev.yml").read_text()
checked += 1
require("MAIDAN_ENABLE_BOOTSTRAP=0" in dev,
        "deploy-dev.yml must build with MAIDAN_ENABLE_BOOTSTRAP=0")
require(dev.count("GITHUB_SHA is not a full commit sha") >= 1
        and "^[0-9a-f]{40}$" in dev,
        "deploy-dev.yml must validate GITHUB_SHA before push/deploy")

sdk = Path(".github/workflows/sdk-release.yml").read_text()
checked += 1
require("tag-shape:" in sdk or "name: release tag shape" in sdk,
        "sdk-release.yml must define a tag-shape guard job")
require("sdk-(ts|py|rs|go)-v" in sdk,
        "sdk-release.yml must require sdk-<lang>-vN.N.N tags")
require(sdk.count("needs: [tag-shape]") >= 4,
        "sdk-release.yml publish jobs must need tag-shape")
require("not a Go sdk release tag" in sdk,
        "sdk-release.yml must re-check the Go module tag shape before push")

alloy = Path("deploy/observability/alloy/config.alloy").read_text()
checked += 1
for key in ("prompt", "diff", "rendered", "body", "content",
            "authorization", "api_key", "password", "token", "MAIDAN_CONTENT_KEK"):
    require(f'key    = "{key}"' in alloy or f"key    = \"{key}\"" in alloy,
            f"alloy config must redact span attribute {key}")

cm = Path("scripts/clean-machine-install.sh").read_text()
checked += 1
require("not a release tag" in cm,
        "clean-machine-install.sh must refuse a non-vN.N.N tag")
require("refusing path" in cm,
        "clean-machine-install.sh must refuse path traversal in fetched files")

np = Path("helm/maidan/templates/networkpolicy.yaml").read_text()
checked += 1
require("allowAllEgress must be false" in np,
        "networkpolicy.yaml must fail when allowAllEgress is true")
require("- {}" not in np,
        "networkpolicy.yaml must not render open allow-all egress")

values = Path("helm/maidan/values.yaml").read_text()
checked += 1
require(re.search(r"(?m)^\s*allowAllEgress:\s*false\s*$", values),
        "helm/maidan/values.yaml must default allowAllEgress to false")

ci = Path("helm/maidan/values-ci.yaml").read_text()
checked += 1
require(re.search(r"(?m)^\s*enabled:\s*true\s*$", ci) and "networkPolicy:" in ci,
        "values-ci.yaml must enable networkPolicy")
require(re.search(r"(?m)^\s*allowAllEgress:\s*false\s*$", ci),
        "values-ci.yaml must set allowAllEgress false")

if errors:
    print("deploy-dev-sdk-alloy gates:", file=sys.stderr)
    for e in errors:
        print(f"  {e}", file=sys.stderr)
    sys.exit(1)
print(f"deploy-dev-sdk-alloy gates: {checked} check(s) ok")
PY
