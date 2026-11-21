### Signing in from Docker Desktop

- **Added:** maidan.world and dev.maidan.world accept `https://mcp.docker.com/oauth/callback` as a redirect for a self-registered OAuth client (`MAIDAN_OAUTH_DCR_REDIRECT_HOSTS`), the callback Docker Desktop's MCP Toolkit registers for a server in its catalog. Until now that registration was refused, so the Toolkit could not sign a person in.
- **Added:** `listings/docker/remote/` holds the Docker MCP Catalog entry in the shape the catalog takes a hosted server today: a remote `server.yaml` for `https://maidan.world/mcp/streamable` with OAuth, an empty `tools.json` and a `readme.md`.
