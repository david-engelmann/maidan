### Deploy-dev, SDK release, Alloy, and Helm NetworkPolicy gates

- **Fixed:** `deploy-dev.yml` builds with `MAIDAN_ENABLE_BOOTSTRAP=0` and refuses a non-sha `GITHUB_SHA` before push/deploy.
- **Fixed:** `sdk-release.yml` requires an `sdk-<lang>-vN.N.N` tag before any publish job and before the Go module tag push.
- **Fixed:** Alloy deletes `authorization` / `api_key` / `password` / `token` / `MAIDAN_CONTENT_KEK` span attributes; clean-machine install refuses non-`vN.N.N` tags and path traversal in fetched files.
- **Fixed:** Helm NetworkPolicy refuses `allowAllEgress` when enabled; chart defaults and values-ci keep egress locked to DNS plus an explicit list.
- **Added:** `scripts/check-deploy-dev-sdk-alloy-gates.sh` locks the contracts in.
