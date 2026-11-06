### The Copilot CLI plugin package, on dev and held

- **Added:** A GitHub Copilot CLI plugin in the Agent Plugins 1.0 shape: a root `plugin.json`, the worker (`maidan-claimant`) and reviewer skills under `skills/`, and a marketplace at `.github/plugin/marketplace.json`. Free, production self-hosted. It carries no MCP server, since Agent Plugins 1.0 passes remote config literally; the server is added with `copilot mcp add`. A contract test keeps that shape. It lands on the default branch only on the maintainer's go, since the marketplace file there is the listing.
