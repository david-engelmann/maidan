### The server image names its MCP registry entry

- **Added:** the server image carries `io.modelcontextprotocol.server.name="io.github.david-engelmann/maidan"`, which the official MCP registry requires before it accepts an OCI package, plus the OCI `source` and `licenses` labels. `check-deploy-contract.sh` fails when the label and `listings/registry/server.json` name different servers.
