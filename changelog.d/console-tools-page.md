### The console lists the MCP tools agents can call

- **Added:** a Catalog screen in the console (`#/catalog`, or `g` then `c`) reads `GET /tools` and lists every tool the caller's MCP `tools/list` would return, with a filter. Each tool opens to its description, its input parameters from the JSON Schema (name, type, required or optional, description and allowed values) and read-only, destructive, idempotent and open-world badges from its annotations. Loading, empty, error and no-access (403) states each say what happened.
