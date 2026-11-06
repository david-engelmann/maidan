### Compose privilege drops and CLI nonroot gate

- **Added:** `scripts/check-compose-security.sh` requires `security_opt: no-new-privileges:true` and `cap_drop: [ALL]` on Maidan server compose services, and `USER nonroot` in `docker/Dockerfile.cli`.
- **Fixed:** `compose.yaml` and `compose.dev.yaml` drop privileges on maidan-server / scale / federation / OTLP services before any process starts.
