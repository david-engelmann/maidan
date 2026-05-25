# HTTP API reference

Maidan serves a machine-readable OpenAPI document at **`GET /openapi.json`**
(OpenAPI 3.0) on any running `maidan-server` instance. Import it into Swagger UI,
Redoc, or your client generator.

The spec documents REST routes and `application/problem+json` errors. MCP
(`POST /mcp`) and WebSocket (`GET /ws/subscribe`) are described in the vault and
MCP tool reference (Track W.3), not fully in OpenAPI.

See also [Production](../docs/Production.md) for probes, environment variables, and bootstrap.
