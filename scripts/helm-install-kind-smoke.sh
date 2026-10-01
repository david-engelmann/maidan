#!/usr/bin/env bash
# Helm install smoke on kind: build the images, install the maidan chart and
# curl /health, then install maidan-stack with its own Postgres and MinIO and
# wait for /health/ready (migrations ran on pgvector, the bucket answers).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="${root}/helm/maidan"
values="${chart}/values-ci.yaml"
stack="${root}/helm/maidan-stack"
stack_values="${stack}/values-ci.yaml"
cluster="${KIND_CLUSTER_NAME:-maidan-helm-smoke}"
release="${HELM_RELEASE:-maidan}"
stack_release="${HELM_STACK_RELEASE:-stack}"
image="${MAIDAN_IMAGE:-maidan-server:dev}"
postgres_image="${MAIDAN_POSTGRES_IMAGE:-maidan-postgres:dev}"
local_port="${HELM_SMOKE_LOCAL_PORT:-18080}"
stack_local_port="${HELM_STACK_SMOKE_LOCAL_PORT:-18081}"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 not installed" >&2
    exit 1
  fi
}

need docker
need helm
need kubectl
need kind
need curl
command -v jq >/dev/null 2>&1 || need jq

# The installs name the images this script loads, so MAIDAN_IMAGE and
# MAIDAN_POSTGRES_IMAGE (name:tag) can point at other builds.
for ref in "${image}" "${postgres_image}"; do
  [[ "${ref##*/}" == *:* ]] || { echo "${ref} has no tag" >&2; exit 1; }
done

pf_pids=()
cleanup() {
  if [[ "${#pf_pids[@]}" -gt 0 ]]; then
    kill "${pf_pids[@]}" 2>/dev/null || true
  fi
  kubectl delete pod -l "app.kubernetes.io/instance=${release}" --force --grace-period=0 2>/dev/null || true
  kind delete cluster --name "${cluster}" 2>/dev/null || true
}
trap cleanup EXIT

# diagnose <namespace>: what a failed install left behind.
diagnose() {
  kubectl get pods,jobs,pvc -n "$1" -o wide 2>/dev/null || true
  kubectl describe pods -n "$1" 2>/dev/null | tail -n 60 || true
  local pod
  for pod in $(kubectl get pods -n "$1" -o name 2>/dev/null); do
    echo "--- logs ${pod}"
    kubectl logs -n "$1" "${pod}" --all-containers --tail=60 2>/dev/null || true
  done
}

# forward <namespace> <service> <local port>
forward() {
  echo "==> port-forward -n $1 svc/$2 :$3"
  kubectl port-forward -n "$1" "svc/$2" "$3:8080" >"/tmp/maidan-helm-pf-$3.log" 2>&1 &
  pf_pids+=("$!")
  sleep 2
}

if kind get clusters 2>/dev/null | grep -qx "${cluster}"; then
  kind delete cluster --name "${cluster}"
fi

echo "==> kind cluster ${cluster}"
kind create cluster --name "${cluster}" --wait 120s

if [[ "${SKIP_DOCKER_BUILD:-}" != "1" ]]; then
  echo "==> docker build ${image}"
  docker build -t "${image}" -f "${root}/crates/maidan-server/Dockerfile" "${root}"
  echo "==> docker build ${postgres_image}"
  docker build -t "${postgres_image}" -f "${root}/docker/Dockerfile.db" "${root}"
else
  echo "==> SKIP_DOCKER_BUILD=1 (using existing ${image} and ${postgres_image})"
fi

echo "==> kind load images"
kind load docker-image "${image}" --name "${cluster}"
kind load docker-image "${postgres_image}" --name "${cluster}"

echo "==> helm install ${release}"
# A fresh content KEK per install, as production would pass one from its
# secret manager; the chart refuses to render without it.
if ! helm install "${release}" "${chart}" \
  -f "${values}" \
  --set image.repository="${image%:*}" --set image.tag="${image##*:}" \
  --set contentKek="$(openssl rand -hex 32)" \
  --namespace maidan \
  --create-namespace \
  --wait \
  --timeout 8m; then
  echo "::error::helm install --wait failed"
  diagnose maidan
  exit 1
fi

service_name="$(
  kubectl get svc -n maidan -l "app.kubernetes.io/instance=${release}" \
    -o jsonpath='{.items[0].metadata.name}'
)"
if [[ -z "${service_name}" ]]; then
  echo "::error::no Service for release ${release} in namespace maidan" >&2
  kubectl get svc -n maidan 2>/dev/null || true
  exit 1
fi
forward maidan "${service_name}" "${local_port}"

echo "==> GET /health"
healthy=""
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${local_port}/health" >/tmp/maidan-helm-health.json; then
    jq -e '.status == "ok"' /tmp/maidan-helm-health.json
    healthy=1
    break
  fi
  sleep 2
done
if [[ -z "${healthy}" ]]; then
  echo "::error::health check failed"
  diagnose maidan
  exit 1
fi
echo "helm install kind smoke OK (${release})"

# The stack, on the same server image, with the bundled Postgres (the pgvector
# image built above) and MinIO. --wait covers the StatefulSets and the
# server's readiness, then the bucket Job (a post-install hook) must finish.
echo "==> helm install ${stack_release} (maidan-stack, postgresql + minio)"
if ! helm install "${stack_release}" "${stack}" \
  -f "${stack_values}" \
  --set maidan.image.repository="${image%:*}" --set maidan.image.tag="${image##*:}" \
  --set postgresql.image.repository="${postgres_image%:*}" \
  --set postgresql.image.tag="${postgres_image##*:}" \
  --set maidan.contentKek="$(openssl rand -hex 32)" \
  --namespace maidan-stack \
  --create-namespace \
  --wait \
  --timeout 10m; then
  echo "::error::helm install of maidan-stack --wait failed"
  diagnose maidan-stack
  exit 1
fi

stack_service="$(
  kubectl get svc -n maidan-stack \
    -l "app.kubernetes.io/name=maidan,app.kubernetes.io/instance=${stack_release}" \
    -o jsonpath='{.items[0].metadata.name}'
)"
if [[ -z "${stack_service}" ]]; then
  echo "::error::no server Service for release ${stack_release} in namespace maidan-stack" >&2
  kubectl get svc -n maidan-stack 2>/dev/null || true
  exit 1
fi
forward maidan-stack "${stack_service}" "${stack_local_port}"

echo "==> GET /health/ready (stack)"
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${stack_local_port}/health/ready" >/tmp/maidan-helm-stack-ready.json; then
    cat /tmp/maidan-helm-stack-ready.json
    echo
    jq -e '.status == "ok" and .db == "ok" and .storage == "ok"' /tmp/maidan-helm-stack-ready.json
    echo "helm install kind smoke OK (${stack_release}: Postgres + MinIO)"
    exit 0
  fi
  sleep 2
done

echo "::error::/health/ready on the stack never answered 200"
curl -s "http://127.0.0.1:${stack_local_port}/health/ready" || true
diagnose maidan-stack
exit 1
