# HTTP API reference

Maidan serves a machine-readable OpenAPI document at **`GET /openapi.json`**
(OpenAPI 3.1) on any running `maidan-server` instance. Import it into Swagger UI,
Redoc, or your client generator.

The spec documents REST routes and their errors. Every client error is an RFC
9457 `application/problem+json` body, including a request the server cannot
read: a malformed path parameter, query string or JSON body is a 400, a body
not sent as `application/json` a 415, a body over `MAIDAN_MAX_BODY_BYTES` a
413, an unknown route a 404 and a wrong method a 405. MCP
(`POST /mcp`) and WebSocket (`GET /ws/subscribe`) are not fully in OpenAPI; see
[MCP reference](./mcp-reference.md).

See [Integrating with Maidan](docs/Integration.md) for auth, capabilities, and transports.

See also [Production](docs/Production.md) for probes, environment variables, and bootstrap.
