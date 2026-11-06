#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# grep is always available; exit 1 (no matches) is success for this scan.
set +e
minio_latest=$(grep -RInE --exclude-dir='.git' 'quay\.io/minio/(minio|mc):latest'   compose.yaml compose.dev.yaml k8s helm 2>/dev/null)
minio_rc=$?
set -e
if [ "$minio_rc" -gt 1 ]; then
  echo "deploy contract: grep failed ($minio_rc) scanning for MinIO :latest; refusing to pass" >&2
  exit 1
fi
if [ "$minio_rc" -eq 0 ]; then
  echo "$minio_latest" >&2
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
  # Portable scan (no rg -P): list every cgr.dev/chainguard/<name> reference that
  # is not a full sha256 digest. Fail closed if the scanner itself errors.
  loose="$(
    NAME="$name" python3 - <<'PY'
import os, re, sys
from pathlib import Path
name = os.environ["NAME"]
pat = re.compile(rf"cgr\.dev/chainguard/{re.escape(name)}(?![-\w])[^\"'\s]*")
ok = re.compile(rf"^cgr\.dev/chainguard/{re.escape(name)}@sha256:[0-9a-f]{{64}}$")
roots = ["compose.yaml", "compose.dev.yaml", "k8s"]
bad = []
for root in roots:
    p = Path(root)
    paths = [p] if p.is_file() else sorted(p.rglob("*")) if p.is_dir() else []
    for path in paths:
        if not path.is_file():
            continue
        try:
            text = path.read_text(errors="replace")
        except OSError as e:
            print(f"scan failed reading {path}: {e}", file=sys.stderr)
            sys.exit(2)
        for m in pat.finditer(text):
            ref = m.group(0)
            if not ok.match(ref):
                bad.append(ref)
sys.stdout.write("\n".join(bad))
PY
  )"
  loose_rc=$?
  if [[ "$loose_rc" -ne 0 ]]; then
    echo "deploy contract: failed scanning cgr.dev/chainguard/${name} references" >&2
    exit 1
  fi
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
      set +e
      found=$(grep -RhoE "cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}"         compose.yaml compose.dev.yaml k8s 2>/dev/null)
      grc=$?
      set -e
      if [ "$grc" -gt 1 ]; then
        echo "deploy contract: grep failed ($grc) listing digests for ${name}" >&2
        exit 1
      fi
      printf '%s\n' "$found"
      echo "${chart_pin}"
    } | awk 'NF' | sort -u
  )"
  if [[ "$(grep -c . <<<"${pins}")" -ne 1 ]]; then
    echo "cgr.dev/chainguard/${name} must be pinned to one digest across compose.yaml, compose.dev.yaml, k8s/ and ${stack_values}; found:" >&2
    echo "${pins}" >&2
    exit 1
  fi
done

# The release image builds without default features (no bootstrap). It must
# still name `otel`, or OTLP_ENDPOINT is silently ignored in every deploy.
for line in $(grep -n -- '--no-default-features' crates/maidan-server/Dockerfile | cut -d: -f1); do
  if ! sed -n "${line}p" crates/maidan-server/Dockerfile | grep -q -- '--features otel'; then
    echo "crates/maidan-server/Dockerfile:${line} builds without default features but drops otel" >&2
    exit 1
  fi
done

# Bootstrap stays off in every non-demo image. The Dockerfile defaults it off;
# release.yml passes it explicitly per leg: 0 for every stock leg, 1 only for
# the -demo leg, whose tag is never latest.
grep -q '^ARG MAIDAN_ENABLE_BOOTSTRAP=0$' crates/maidan-server/Dockerfile \
  || { echo "crates/maidan-server/Dockerfile must default MAIDAN_ENABLE_BOOTSTRAP to 0" >&2; exit 1; }
python3 - <<'PY' || exit 1
import sys, yaml
wf = yaml.safe_load(open(".github/workflows/release.yml"))
job = wf["jobs"]["docker-server"]
legs = job["strategy"]["matrix"]["include"]
def fail(msg):
    print(f"release.yml docker-server: {msg}", file=sys.stderr); sys.exit(1)
stock = [l for l in legs if l.get("suffix", "") == ""]
if not stock or any(str(l.get("bootstrap")) != "0" for l in stock):
    fail("every stock leg must set bootstrap: \"0\"")
if any(str(l.get("bootstrap")) != "0" and l.get("suffix") != "-demo" for l in legs):
    fail("only the -demo leg may build with bootstrap")
step = next(s for s in job["steps"] if s.get("name") == "build + push maidan-server")
if "MAIDAN_ENABLE_BOOTSTRAP=${{ matrix.bootstrap }}" not in step["with"]["build-args"]:
    fail("the build must pass MAIDAN_ENABLE_BOOTSTRAP=${{ matrix.bootstrap }}")
manifest = "\n".join(s.get("run", "") for s in wf["jobs"]["docker-server-manifest"]["steps"])
for cmd in manifest.split("docker buildx imagetools create")[1:]:
    if "-demo" in cmd and ":latest" in cmd:
        fail("the -demo image must never be tagged latest")
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

echo "deployment contract OK"
