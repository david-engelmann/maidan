### Docker nginx / DB / OTEL harden and Helm values schema

- **Fixed:** `docker/nginx-scale.conf` turns off `server_tokens`, caps body size, adds frame/content-type/referrer headers, and hides upstream `Server` / `X-Powered-By`.
- **Fixed:** `docker/Dockerfile.db` modes `/archive` to `0700` and wipes `docker-entrypoint-initdb.d` so a base bump cannot reintroduce bundled schema.
- **Fixed:** `docker/otel-collector-config.yaml` runs a `memory_limiter` on trace and metric pipelines and caps gRPC recv size.
- **Added:** `helm/maidan/values.schema.json` requires non-root podSecurityContext and memory requests/limits; `scripts/check-docker-nginx-helm-gates.sh` locks the contracts in.
