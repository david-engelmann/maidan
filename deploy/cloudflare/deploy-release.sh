#!/usr/bin/env bash
# Deploys one release tag to maidan.world and waits until /health reports it.
# deploy-prod runs it for the new tag, and again for the tag that was running
# when the new one fails its checks. Needs CLOUDFLARE_API_TOKEN and
# CLOUDFLARE_ACCOUNT_ID; runs from deploy/cloudflare.
set -euo pipefail
tag=${1:?usage: deploy-release.sh vN.N.N}
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a release tag: $tag" >&2; exit 1; }
src=ghcr.io/david-engelmann/maidan-server

# The release workflow signs each image keylessly; refuse an image it did not
# sign, so a re-pointed tag cannot reach production.
cosign verify "$src:$tag" \
  --certificate-identity-regexp '^https://github\.com/david-engelmann/maidan/\.github/workflows/release\.yml@refs/(tags/v[0-9]+\.[0-9]+\.[0-9]+|heads/main)$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com > /dev/null

# Pull the linux/amd64 manifest by digest: a multi-platform pull keeps the
# index, whose platform wrangler cannot read.
digest=$(docker buildx imagetools inspect "$src:$tag" --raw \
  | jq -r '.manifests[] | select(.platform.os=="linux" and .platform.architecture=="amd64") | .digest')
docker pull -q "$src@$digest" > /dev/null
docker tag "$src@$digest" "maidan-server:$tag"
# The push reads a Wrangler config, and the dev one names a Dockerfile build.
npx wrangler containers push -c wrangler.prod.jsonc "maidan-server:$tag"

# A copy beside the original, so its relative paths resolve the same and a
# second run (the rollback) starts from the placeholder again.
cfg=wrangler.release.jsonc
trap 'rm -f "$cfg"' EXIT
image="registry.cloudflare.com/$CLOUDFLARE_ACCOUNT_ID/maidan-server:$tag"
sed "s#\"image\": \"registry.cloudflare.com/ACCOUNT/maidan-server:RELEASE\"#\"image\": \"$image\"#" wrangler.prod.jsonc > "$cfg"
grep -q "\"image\": \"$image\"" "$cfg"
# The first deploy can look for the Durable Object namespace before it is
# visible (DURABLE_OBJECT_NOT_FOUND); the second finds it.
npx wrangler deploy -c "$cfg" || { sleep 15; npx wrangler deploy -c "$cfg"; }

got=""
for _ in $(seq 1 36); do
  got=$(curl -fsS https://maidan.world/health | jq -r .version || true)
  if [ "$got" = "$tag" ]; then echo "maidan.world runs $tag"; break; fi
  sleep 10
done
[ "$got" = "$tag" ] || { echo "maidan.world answers ${got:-nothing}, not $tag" >&2; exit 1; }

# An anonymous caller reads nothing: a production instance with auth off must
# never pass.
code=$(curl -s -o /dev/null -w '%{http_code}' https://maidan.world/me)
[ "$code" = 401 ] || { echo "GET /me without a token answered $code, not 401" >&2; exit 1; }
