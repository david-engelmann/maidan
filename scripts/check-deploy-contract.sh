#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if rg -n 'quay\.io/minio/(minio|mc):latest' \
  compose.yaml compose.dev.yaml k8s helm; then
  echo "MinIO deployment images must use immutable release tags, not :latest" >&2
  exit 1
fi

# Chainguard publishes MinIO only as :latest, so every deploy path pins it by
# digest, and they pin the same one: a bump in compose that leaves the stack
# chart or k8s on the old build is a split nobody chose. The chart keeps the
# repository and digest on separate lines of its values.
stack_values=helm/maidan-stack/values.yaml
for name in minio minio-client; do
  chart_pin="$(awk -v repo="cgr.dev/chainguard/${name}" '
    $1 == "repository:" { cur = ($2 == repo) }
    cur && $1 == "digest:" { print repo "@" $2; cur = 0 }
  ' "${stack_values}")"
  [[ -n "${chart_pin}" ]] || {
    echo "${stack_values} pins no digest for cgr.dev/chainguard/${name}; if it moved, update this script" >&2
    exit 1
  }
  # Any reference to the image that is not a full digest (a tag, or none) is
  # refused before the pins are compared, or it would silently drop out of them.
  loose="$(rg -oP --no-filename "cgr\.dev/chainguard/${name}(?![-\w])[^\"'\s]*" \
    compose.yaml compose.dev.yaml k8s | rg -v "^cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}$" || true)"
  if [[ -n "${loose}" ]]; then
    echo "cgr.dev/chainguard/${name} must be referenced by a full sha256 digest; found:" >&2
    echo "${loose}" >&2
    exit 1
  fi
  [[ "${chart_pin}" =~ ^cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}$ ]] || {
    echo "${stack_values}: ${chart_pin} is not a full sha256 digest" >&2
    exit 1
  }
  pins="$(
    {
      rg -o --no-filename "cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}" \
        compose.yaml compose.dev.yaml k8s || true
      echo "${chart_pin}"
    } | sort -u
  )"
  if [[ "$(grep -c . <<<"${pins}")" -ne 1 ]]; then
    echo "cgr.dev/chainguard/${name} must be pinned to one digest across compose.yaml, compose.dev.yaml, k8s/ and ${stack_values}; found:" >&2
    echo "${pins}" >&2
    exit 1
  fi
done

# No build that ships turns bootstrap on: its seed routes are a test-only
# feature. The server image names its features rather than take the crate's
# defaults, and must still name `otel`, or OTLP_ENDPOINT is silently ignored in
# every deploy.
dockerfile=crates/maidan-server/Dockerfile
builds="$(grep -nE 'cargo (build|chef cook)' "${dockerfile}" || true)"
[[ -n "${builds}" ]] || { echo "${dockerfile} has no cargo build" >&2; exit 1; }
while IFS= read -r build; do
  if ! grep -q -- '--no-default-features --features otel' <<<"${build}"; then
    echo "${dockerfile}:${build%%:*} must build with --no-default-features --features otel" >&2
    exit 1
  fi
done <<<"${builds}"
if grep -niE '^[^#]*bootstrap' "${dockerfile}"; then
  echo "${dockerfile} must not name bootstrap: no image builds it" >&2
  exit 1
fi
# The crate's defaults are what every other build (CI's debug server, the demo
# and SDK scripts) gets.
python3 - <<'PY' || exit 1
import sys, tomllib
features = tomllib.load(open("crates/maidan-server/Cargo.toml", "rb"))["features"]
if "bootstrap" in features.get("default", []):
    sys.exit("crates/maidan-server/Cargo.toml: bootstrap must not be a default feature")
PY
# Only a test turns the feature on (the strip job's bootstrap_e2e); no workflow
# builds an image or binary with it, or passes the old build argument.
offenders="$(grep -rnE 'MAIDAN_ENABLE_BOOTSTRAP|features[= ][^#]*bootstrap' .github/workflows \
  | grep -vE 'cargo test ' || true)"
if [[ -n "${offenders}" ]]; then
  echo "a workflow builds with bootstrap; only cargo test may enable it:" >&2
  echo "${offenders}" >&2
  exit 1
fi
python3 - <<'PY' || exit 1
import sys, yaml
wf = yaml.safe_load(open(".github/workflows/release.yml"))
job = wf["jobs"]["docker-server"]
def fail(msg):
    print(f"release.yml docker-server: {msg}", file=sys.stderr); sys.exit(1)
for leg in job["strategy"]["matrix"]["include"]:
    if set(leg) & {"bootstrap", "suffix"}:
        fail(f"a leg builds a second image ({leg}); every leg is the one stock image")
step = next(s for s in job["steps"] if s.get("name") == "build + push maidan-server")
if "BOOTSTRAP" in step["with"].get("build-args", ""):
    fail("the build must not pass a bootstrap build argument")
PY

# A release created with GITHUB_TOKEN does not emit `release: published`.
# release.yml has to dispatch every workflow that listens for that event.
# sdk-release.yml must not be one of them: it publishes on its own
# sdk-*-v* tags. deploy-demo.yml keeps the only DEMO_DOMAIN switch.
python3 - <<'PY' || exit 1
import pathlib, sys, yaml
root = pathlib.Path(".github/workflows")
release = yaml.safe_load((root / "release.yml").read_text())

def fail(msg):
    print(f"release.yml follow-ups: {msg}", file=sys.stderr)
    sys.exit(1)

def trigger(wf):
    # PyYAML reads the workflow key `on` as boolean True.
    return wf.get(True) or wf.get("on") or {}

runs = []
for job in release["jobs"].values():
    for step in job.get("steps") or []:
        run = step.get("run") or ""
        if "gh workflow run" in run:
            runs.append(run)
blob = "\n".join(runs)

listeners = []
for path in sorted(root.glob("*.yml")):
    wf = yaml.safe_load(path.read_text())
    types = (trigger(wf).get("release") or {}).get("types") or []
    if "published" in types:
        listeners.append(path.name)
expected = ["clean-install.yml", "deploy-demo.yml", "deploy-prod.yml"]
if listeners != expected:
    fail(
        "workflows listening for release: published are "
        + ", ".join(listeners)
        + "; expected "
        + ", ".join(expected)
        + ". A new listener needs a dispatch from release.yml, or it should not use this event."
    )

for name in expected:
    if f"gh workflow run {name}" not in blob:
        fail(f"must dispatch {name}")
    chunk = blob.split(f"gh workflow run {name}", 1)[1].split("gh workflow run", 1)[0]
    if "--ref main" not in chunk or "-f tag=" not in chunk:
        fail(f"{name} must be dispatched with --ref main and -f tag=")

if "sdk-release" in blob:
    fail("must not dispatch sdk-release.yml")

for job_name, workflow in (
    ("deploy-prod", "deploy-prod.yml"),
    ("deploy-demo", "deploy-demo.yml"),
    ("clean-install", "clean-install.yml"),
):
    job = release["jobs"].get(job_name) or {}
    if job.get("needs") != ["release"]:
        fail(f"{job_name} must run after the release job")
    if (job.get("permissions") or {}).get("actions") != "write":
        fail(f"{job_name} needs actions: write to dispatch {workflow}")
    if job.get("if"):
        fail(f"{job_name} must not add its own if; the target workflow decides")

demo = yaml.safe_load((root / "deploy-demo.yml").read_text())
if demo["jobs"]["deploy"].get("if") != "vars.DEMO_DOMAIN == 'true'":
    fail("deploy-demo.yml must skip its deploy unless vars.DEMO_DOMAIN == 'true'")
PY

# The registry manifest and the image label must name the same server.
name=$(python3 -c 'import json;print(json.load(open("listings/registry/server.json"))["name"])')
grep -q "io.modelcontextprotocol.server.name=\"$name\"" crates/maidan-server/Dockerfile \
  || { echo "crates/maidan-server/Dockerfile must label io.modelcontextprotocol.server.name=\"$name\"" >&2; exit 1; }

# The prod overlay runs the server in production mode: without MAIDAN_ENV the
# server's own refusals (the OIDC mock, AUTH_DISABLED, fault injection) are off.
# The Helm chart's half of this is in helm-template-smoke.sh.
grep -A2 -x '        path: /data/MAIDAN_ENV' k8s/overlays/prod/kustomization.yaml \
  | grep -qx '        value: production' \
  || { echo "k8s/overlays/prod must set MAIDAN_ENV=production in maidan-config" >&2; exit 1; }

echo "deployment contract OK"
