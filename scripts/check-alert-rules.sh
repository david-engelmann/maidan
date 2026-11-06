#!/usr/bin/env bash
# Validate the Maidan SLO rules with promtool: lint the PromQL/templates and run
# the unit tests. The rules ship as a Kubernetes PrometheusRule CRD, so we first
# extract `.spec` (the raw `groups:` document promtool expects) into a temporary,
# git-ignored file. Used by CI (`promtool (alert rules)`, which is a required
# check) and locally. The sole rule validator — metric-name presence is also
# guarded by the `alert_templates_contract` test.
set -euo pipefail

cd "$(dirname "$0")/.."

RULES="docs/alerts/prometheus-rules-maidan-slo.yaml"
TEST_FILE="prometheus-rules-maidan-slo.test.yaml"
GEN="docs/alerts/slo-rules.generated.yaml"

# Missing rule files must fail even when promtool is absent: a soft skip would
# hide a deleted or moved CRD from local runs and from CI image drift.
[[ -f "$RULES" ]] || { echo "alert rules: missing $RULES" >&2; exit 1; }
[[ -f "docs/alerts/$TEST_FILE" ]] || { echo "alert rules: missing docs/alerts/$TEST_FILE" >&2; exit 1; }

if ! command -v promtool >/dev/null 2>&1; then
  echo "promtool not found — install it (https://prometheus.io/download/) to" \
       "validate the SLO rules locally; CI runs it in the required" \
       "'promtool (alert rules)' job. Skipping." >&2
  exit 0
fi

cleanup() { rm -f "$GEN"; }
trap cleanup EXIT

# Extract the raw rule groups from the CRD's .spec. Prefer yq (present on CI
# runners); fall back to python3 + PyYAML (present locally).
if command -v yq >/dev/null 2>&1; then
  yq '.spec' "$RULES" >"$GEN"
else
  python3 -c "import yaml; yaml.safe_dump(yaml.safe_load(open('$RULES'))['spec'], open('$GEN', 'w'), sort_keys=False)"
fi
# Refuse an empty or non-groups extract so a bad CRD cannot pass check rules.
# Prefer yq when that is how we extracted, so a yq-only runner does not need PyYAML.
if command -v yq >/dev/null 2>&1; then
  groups="$(yq '.groups | length' "$GEN" 2>/dev/null || echo 0)"
  [[ "$groups" =~ ^[1-9][0-9]*$ ]] || {
    echo "alert rules: extracted $GEN has no groups" >&2
    exit 1
  }
else
  python3 - <<PY
import sys, yaml
doc = yaml.safe_load(open("$GEN"))
groups = (doc or {}).get("groups") if isinstance(doc, dict) else None
if not isinstance(groups, list) or not groups:
    print("alert rules: extracted $GEN has no groups", file=sys.stderr)
    sys.exit(1)
PY
fi

promtool check rules "$GEN"

# rule_files in the test fixture are resolved relative to the working directory,
# so run from docs/alerts where the generated file lives.
( cd docs/alerts && promtool test rules "$TEST_FILE" )

echo "alert rules: check + unit tests passed"
