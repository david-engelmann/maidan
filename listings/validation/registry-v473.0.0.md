# Validation record: the official MCP registry at v473.0.0

This record was written on 2026-11-22, after the entry was sent. The template
asks for it before. The release it rests on had its release-level record
first (`listings/validation/v473.md`), and what this listing's own record adds
is below, with what was not done named under "Still open".

| | |
|---|---|
| Listing | the official MCP registry, `listings/registry/server.json` |
| Release | `v473.0.0` |
| Commit | `77e31d8bf491f0db6e28fb64c09f364da4553781` |
| Server image | `ghcr.io/david-engelmann/maidan-server:v473.0.0`, index digest `sha256:9042e9997fa31ea5abd0a10a1bd1478a53affdcfdaf70083b80b7c5f7313fdd3`, cosign signature verified against `release.yml@refs/tags/v473.0.0`. Both `linux/amd64` and `linux/arm64` carry `io.modelcontextprotocol.server.name=io.github.david-engelmann/maidan` |
| Binaries | `maidan-aarch64-apple-darwin.tar.gz`, sha256 `4d9dd5cb4f6d93331191e18b3ee4d9bfa837303aae9d1c4dfab150164ffa9c62`, cosign bundle verified against `release.yml@refs/tags/v473.0.0`. `SHA256SUMS` (sha256 `ef709cdaf3bcf503aaf18d3a114f9093a20dff24fbf6d4e0ef5c60cad9cab4b8`) is cosign-verified the same way, and the tarball matches its line. Extracted: `maidan-server` sha256 `d3deb7c601f3cf6afc5be73beeb836c3b378db2e12b832aaafdbca5958809c42`, `maidan` sha256 `76e3064b12600cf7b2742315cb6631803c40769dd0664bee0c1d4af7df0249cd` (`maidan --version` prints `maidan v473.0.0`) |
| Listed surface | the full MCP catalog, 265 tools, on the hosted endpoint `https://maidan.world/mcp/streamable` and in the server image |
| Run | 2026-11-21 and 2026-11-22 UTC, by a Claude Code agent: the release record's sweep from the signed binaries, and the `publish MCP registry` workflow dispatched with `release=v473.0.0` after the launch gate passed on that release |
| Client | the registry's own publisher, `mcp-publisher` v1.8.7, unmodified, signing in with GitHub's OIDC token for the `io.github.david-engelmann` namespace |
| Result | **655 checks passed, none failed**: "every tool call passed (265 tools in the catalog), the 6 that need a backend included" (the release's own sweep; see `listings/validation/v473.md`) |

## Sent

Published on 2026-11-22 by run 44979949080 of the workflow. The registry lists `io.github.david-engelmann/maidan` at 473.0.0, with the remote `https://maidan.world/mcp/streamable` and the package `ghcr.io/david-engelmann/maidan-server:v473.0.0`. A first publish on 2026-11-21, naming `v472.0.0`, was refused: the registry takes an OCI package as one reference in `identifier`, which `v473.0.0` corrects.

## Clean install

- Host: a fresh GitHub-hosted runner and the maintainer's Mac (the release record). A clean machine run by another person: not done.
- Steps: `scripts/clean-machine-install.sh` with `MAIDAN_TAG=v473.0.0 STRICT=1`, which pulls the image the entry names.
- Result: installed and answering on both.

## Tools in the listed surface

- Each tool succeeds with valid parameters: pass, for all 265 (the release
  record's sweep).
- Each tool returns an actionable error with bad parameters, naming the
  argument it got wrong: pass, for all 265.
- Every tool carries `readOnlyHint`, `destructiveHint`, `openWorldHint` and a
  title (the annotation contract test passed at this commit): pass.

## Example prompts through the platform's own model

Not run. The registry has no model of its own: it is an index that clients read. The prompts in `listings/prompts.json` for the clients that read it are run under those clients' own records.

## Identity and tokens

- Token the listing authenticates as: none. The entry names a remote that signs in with OAuth and an image a person runs themselves.
- Reviewer token: none; the registry has no review step.
- Never a `token:admin` or non-expiring token: confirmed, the entry carries no token.

## Protocol-layer properties

- No surface lets the supervised agent compose, trigger or satisfy its own
  approval: the launch gate's step 5 on this release (the agent's own close is
  refused for separation of duties), and `approval_decide` sends a person to
  the console.
- Every approval binds to a verified human identity, per-user credentials at
  minimum: a gate is accepted only by a session the person signed in to or by
  `approval:grant`; the launch gate's step 3 checks that consent refuses a
  session made from a token.
- The close gate stays server-side and durable: the launch gate's steps 5 and
  6 on this release.

## Trust model

The listing's materials and tests carry this sentence: Provider-rendered approval UI is untrusted display, the Maidan console is the trust root, and an approval whose evidence the human did not see in the console is provisional.

- Where it appears in the listing: not in the registry entry, whose schema has a title and a 100-character description and no room for it. It is in `docs/Listing.md` and the directory listings' copy.

## Still open before this listing ships

- **This record before the publish:** it was written after. Nothing in it would have stopped the publish.
- **The trust sentence in the entry:** the schema has no field for it.
- **A clean install by another person on another machine:** the maintainer's, as in the release record.
- **Re-publishing on each release:** the entry names an image tag, so a new release needs a new publish (`gh workflow run publish-mcp-registry.yml -f release=<tag>`), after the gate passes on it.
