### Server, CLI, and demo image pin gates

- **Fixed:** `crates/maidan-server/Dockerfile` and `docker/Dockerfile.cli` copy their binaries mode `0555`, declare `STOPSIGNAL SIGTERM`, and keep the CLI MIT license label.
- **Fixed:** `deploy/cloudflare/demo/Dockerfile` requires a digest-pinned `SERVER_IMAGE` and copies the server plus entrypoint mode `0555` before `USER demo`.
- **Fixed:** `deploy-demo.yml` refuses an empty TAG or an empty/malformed linux/amd64 digest after cosign verify; `deploy-prod.yml` and `clean-install.yml` refuse an empty or non-`vN.N.N` tag.
- **Added:** `scripts/check-server-image-gates.sh` locks the contracts in.
