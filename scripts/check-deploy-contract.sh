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

# The registry manifest and the image label must name the same server.
name=$(python3 -c 'import json;print(json.load(open("listings/registry/server.json"))["name"])')
grep -q "io.modelcontextprotocol.server.name=\"$name\"" crates/maidan-server/Dockerfile \
  || { echo "crates/maidan-server/Dockerfile must label io.modelcontextprotocol.server.name=\"$name\"" >&2; exit 1; }

echo "deployment contract OK"
