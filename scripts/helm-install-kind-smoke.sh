#!/usr/bin/env bash
# Helm install smoke on kind (Cluster 55): build image, install chart, curl /health.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="${root}/helm/maidan"
values="${chart}/values-ci.yaml"
cluster="${KIND_CLUSTER_NAME:-maidan-helm-smoke}"
release="${HELM_RELEASE:-maidan}"
image="${MAIDAN_IMAGE:-maidan-server:dev}"
local_port="${HELM_SMOKE_LOCAL_PORT:-18080}"

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

cleanup() {
  kubectl delete pod -l "app.kubernetes.io/instance=${release}" --force --grace-period=0 2>/dev/null || true
  kind delete cluster --name "${cluster}" 2>/dev/null || true
}
trap cleanup EXIT

if kind get clusters 2>/dev/null | grep -qx "${cluster}"; then
  kind delete cluster --name "${cluster}"
fi

echo "==> kind cluster ${cluster}"
kind create cluster --name "${cluster}" --wait 120s

if [[ "${SKIP_DOCKER_BUILD:-}" != "1" ]]; then
  echo "==> docker build ${image}"
  docker build -t "${image}" -f "${root}/crates/maidan-server/Dockerfile" "${root}"
else
  echo "==> SKIP_DOCKER_BUILD=1 (using existing ${image})"
fi

echo "==> kind load image"
kind load docker-image "${image}" --name "${cluster}"

echo "==> helm install ${release}"
if ! helm install "${release}" "${chart}" \
  -f "${values}" \
  --namespace maidan \
  --create-namespace \
  --wait \
  --timeout 8m; then
  echo "::error::helm install --wait failed"
  kubectl get pods -n maidan -o wide 2>/dev/null || true
  kubectl logs -n maidan -l "app.kubernetes.io/instance=${release}" --tail=80 2>/dev/null || true
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
echo "==> port-forward svc/${service_name} :${local_port}"
kubectl port-forward -n maidan "svc/${service_name}" "${local_port}:8080" >/tmp/maidan-helm-pf.log 2>&1 &
pf_pid=$!
trap 'kill "${pf_pid}" 2>/dev/null || true; cleanup' EXIT
sleep 2

echo "==> GET /health"
for i in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${local_port}/health" >/tmp/maidan-helm-health.json; then
    jq -e '.status == "ok"' /tmp/maidan-helm-health.json
    echo "helm install kind smoke OK"
    kill "${pf_pid}" 2>/dev/null || true
    exit 0
  fi
  sleep 2
done

echo "::error::health check failed"
kubectl get pods -n maidan -o wide || true
kubectl logs -n maidan -l "app.kubernetes.io/instance=${release}" --tail=80 || true
exit 1
