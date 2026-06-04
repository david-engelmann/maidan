# External agents integrating with Maidan

If you are connecting **to** a Maidan server (not hacking on this Rust repo), start here.

1. **[docs/Integration.md](docs/Integration.md)** — canonical integration guide (HTTP, MCP, WebSocket, A2A, webhooks, capabilities).
2. **Published mdBook:** [https://david-engelmann.github.io/maidan/](https://david-engelmann.github.io/maidan/) — same content plus generated MCP tool reference.
3. **`GET /openapi.json`** on your deployment — OpenAPI 3.0 for REST.
4. **[docs/Capability Map.md](docs/Capability%20Map.md)** — capability strings and contract file index.

**Edge / Raspberry Pi:** [docs/Pi.md](docs/Pi.md) — install **`v101.0.0`** (ARM64 binary or container).

Do **not** start with `docs/Clusters/` or `docs/Retros/`; those are maintainer planning notes and often use Obsidian `[[wikilinks]]` that GitHub does not render as links.

To contribute to the codebase, read [CLAUDE.md](CLAUDE.md) instead.
