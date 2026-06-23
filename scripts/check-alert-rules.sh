#!/usr/bin/env bash
# Validate the Maidan SLO rules with promtool: lint the PromQL/templates and run
# the unit tests. The rules ship as a Kubernetes PrometheusRule CRD, so we first
# extract `.spec` (the raw `groups:` document promtool expects) into a temporary,
# git-ignored file. Used by CI (`promtool (alert rules)`, which is a required
# check) and locally. The sole rule validator — metric-name presence is also
# guarded by the `alert_templates_contract` test.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v promtool >/dev/null 2>&1; then
  echo "promtool not found — install it (https://prometheus.io/download/) to" \
       "validate the SLO rules locally; CI runs it in the required" \
       "'promtool (alert rules)' job. Skipping." >&2
  exit 0
fi

RULES="docs/alerts/prometheus-rules-maidan-slo.yaml"
TEST_FILE="prometheus-rules-maidan-slo.test.yaml"
GEN="docs/alerts/slo-rules.generated.yaml"

cleanup() { rm -f "$GEN"; }
trap cleanup EXIT

# Extract the raw rule groups from the CRD's .spec. Prefer yq (present on CI
# runners); fall back to python3 + PyYAML (present locally).
if command -v yq >/dev/null 2>&1; then
  yq '.spec' "$RULES" >"$GEN"
else
  python3 -c "import yaml; yaml.safe_dump(yaml.safe_load(open('$RULES'))['spec'], open('$GEN', 'w'), sort_keys=False)"
fi

promtool check rules "$GEN"

# rule_files in the test fixture are resolved relative to the working directory,
# so run from docs/alerts where the generated file lives.
( cd docs/alerts && promtool test rules "$TEST_FILE" )

echo "alert rules: check + unit tests passed"
