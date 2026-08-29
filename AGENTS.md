# External agents integrating with Maidan

If you are connecting **to** a Maidan server (not hacking on this Rust repo), start here.

1. **[docs/Integration.md](docs/Integration.md)** — canonical integration guide (HTTP, MCP, WebSocket, A2A, webhooks, capabilities).
2. **Published site:** [mdBook on GitHub Pages](https://david-engelmann.github.io/maidan/) — same `docs/` Markdown plus generated MCP tool reference. (A `maidan.world` product domain is planned for the public preview but is **not live yet** — see [docs/Promotion.md](docs/Promotion.md); use the GitHub Pages URL today.)
3. **`GET /openapi.json`** on your deployment — OpenAPI 3.0 for REST.
4. **[docs/Capability Map.md](docs/Capability%20Map.md)** — capability strings and contract file index.
5. **[docs/Protocols.md](docs/Protocols.md)** — MCP vs A2A vs REST vs webhooks. MCP is **`2026-07-28`** by default (`2024-11-05` still honored on explicit request); A2A v1.0 over JSON-RPC + REST (gRPC partial).
6. **[docs/Providers.md](docs/Providers.md)** — Postgres/SQLite hosts, S3, embeddings, OIDC.

**Edge / Raspberry Pi:** [docs/Pi.md](docs/Pi.md) — install the latest ARM64 binary or container from the [Releases page](https://github.com/david-engelmann/maidan/releases).

Do **not** start with `docs/Clusters/` or `docs/Retros/`; those are maintainer planning notes and often use Obsidian `[[wikilinks]]` that GitHub does not render as links.

To contribute to the codebase, read [CLAUDE.md](CLAUDE.md) instead.
