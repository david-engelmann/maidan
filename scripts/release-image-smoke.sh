#!/usr/bin/env bash
# Prove the exact images published by a release can initialize and serve a
# fresh authenticated Postgres deployment. This intentionally pulls GHCR tags;
# a locally built image is not evidence for the published artifact.
set -euo pipefail

tag="${1:?usage: release-image-smoke.sh <tag> [repository-owner]}"
owner="${2:-david-engelmann}"
run_suffix="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}-$$"
run_suffix="${run_suffix//[^a-zA-Z0-9_.-]/-}"
network="maidan-release-smoke-${run_suffix}"
postgres="maidan-release-postgres-${run_suffix}"
server="maidan-release-server-${run_suffix}"
registry="ghcr.io/${owner}"
postgres_image="${registry}/maidan-postgres:${tag}"
server_image="${registry}/maidan-server:${tag}"
cli_image="${registry}/maidan-cli:${tag}"
database_url="postgres://maidan:release-smoke-password@${postgres}:5432/maidan"
health_file=""
me_file=""

cleanup() {
  status=$?
  if (( status != 0 )); then
    docker logs "$server" 2>/dev/null || true
    docker logs "$postgres" 2>/dev/null || true
  fi
  test -z "$health_file" || rm -f "$health_file"
  test -z "$me_file" || rm -f "$me_file"
  docker rm -f "$server" "$postgres" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT

for image in "$postgres_image" "$server_image" "$cli_image"; do
  docker pull "$image"
  test -n "$(docker image inspect "$image" --format '{{join .RepoDigests " "}}')"
done

test "$(docker image inspect "$server_image" --format '{{json .Config.Entrypoint}}')" \
  = '["/usr/local/bin/maidan-server"]'
test "$(docker image inspect "$cli_image" --format '{{json .Config.Entrypoint}}')" \
  = '["/usr/local/bin/maidan"]'
test "$(docker image inspect "$cli_image" --format '{{.Config.User}}')" = 'nonroot:nonroot'
test "$(docker image inspect "$cli_image" --format '{{index .Config.Labels "org.opencontainers.image.version"}}')" \
  = "$tag"
test "$(docker run --rm "$cli_image" --version)" = "maidan ${tag}"

docker network create "$network" >/dev/null
docker run -d --name "$postgres" --network "$network" \
  -e POSTGRES_USER=maidan \
  -e POSTGRES_PASSWORD=release-smoke-password \
  -e POSTGRES_DB=maidan \
  --health-cmd='pg_isready -U maidan -d maidan' \
  --health-interval=2s --health-timeout=2s --health-retries=30 \
  "$postgres_image" >/dev/null

for _ in $(seq 1 60); do
  case "$(docker inspect "$postgres" --format '{{.State.Health.Status}}')" in
    healthy) break ;;
    unhealthy) echo 'published Postgres image became unhealthy' >&2; exit 1 ;;
  esac
  sleep 2
done
test "$(docker inspect "$postgres" --format '{{.State.Health.Status}}')" = healthy

# Keep the one-time bearer token out of logs. Its only use is the authenticated
# request below, and the captured initialization output is discarded afterward.
init_output="$(docker run --rm --network "$network" \
  -e DATABASE_URL="$database_url" \
  "$cli_image" init --workspace release-smoke --admin-handle release-smoke-admin)"
admin_token="$(awk '$1 ~ /^maid_/ { print $1 }' <<<"$init_output")"
test "$(wc -w <<<"$admin_token" | tr -d ' ')" = 1
test "${admin_token#maid_}" != "$admin_token"
unset init_output

docker run -d --name "$server" --network "$network" \
  --tmpfs /data:rw,uid=65532,gid=65532,mode=0700 \
  -p 127.0.0.1::8080 \
  -e DATABASE_URL="$database_url" \
  -e ARTIFACT_BACKEND=localfs \
  -e ARTIFACT_LOCALFS_ROOT=/data/artifacts \
  -e MAIDAN_SESSION_SECRET=release-smoke-session-secret-32bytes \
  -e MAIDAN_BIND=0.0.0.0:8080 \
  "$server_image" >/dev/null

host_port="$(docker port "$server" 8080/tcp | awk -F: 'NR == 1 { print $NF }')"
test -n "$host_port"
base_url="http://127.0.0.1:${host_port}"
health_file="$(mktemp)"
me_file="$(mktemp)"
for _ in $(seq 1 60); do
  if curl -fsS "${base_url}/health/ready" -o "$health_file"; then
    break
  fi
  sleep 2
done

jq -e --arg tag "$tag" '.status == "ok" and .version == $tag' "$health_file"
curl -fsS -H "Authorization: Bearer ${admin_token}" "${base_url}/me" -o "$me_file"
jq -e '.is_bearer == true and (.member_id | type == "string") and (.workspace_id | type == "string")' "$me_file"
test "$(curl -sS -o /dev/null -w '%{http_code}' "${base_url}/me")" = 401
echo "published image smoke passed for ${tag}"
