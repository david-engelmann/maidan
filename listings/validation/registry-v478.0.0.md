# Validation record: the official MCP registry at v478.0.0

This record was written on 2026-11-24, before the entry was republished, as
the template asks. The release it rests on has its release-level record
(`listings/validation/v478.md`), and the launch gate passed on it against
maidan.world the same day. What this listing's own record adds is below, with
what was not done named under "Still open".

| | |
|---|---|
| Listing | the official MCP registry, `listings/registry/server.json` |
| Release | `v478.0.0` |
| Commit | `3d3b346758113b823f59d0dad23f0a49b77527f7` |
| Server image | `ghcr.io/david-engelmann/maidan-server:v478.0.0`, index digest `sha256:b0e45c79a640a03dd89da86884b65f66f8cef485010828d3e8cb225f7152abf5`, cosign signature verified against `release.yml@refs/tags/v478.0.0`. Both `linux/amd64` and `linux/arm64` carry `io.modelcontextprotocol.server.name=io.github.david-engelmann/maidan` |
| Binaries | `maidan-aarch64-apple-darwin.tar.gz`, sha256 `4ddb406ec7776fa67ab58d3efaafc8267b7f572fc9e07298098127e35eab58cb`, cosign bundle verified against `release.yml@refs/tags/v478.0.0`. `SHA256SUMS` (sha256 `60ed2d2f6c0b6957e7ebafa5c2e4af04ba784a4174adfe5f05941e77dffad582`) is cosign-verified the same way, and the tarball matches its line. Extracted: `maidan-server` sha256 `3bb9213a79a8a22729ba8b90d54a9ea6000a19e21043b72ea82ddc0a9010b946`, `maidan` sha256 `06321db6dcfcff74da861e5d8d0403a54a4c0999c4f78937f8d63f0c20899170` (`maidan --version` prints `maidan v478.0.0`) |
| Listed surface | the full MCP catalog, 265 tools, on the hosted endpoint `https://maidan.world/mcp/streamable` and in the server image |
| Run | 2026-11-24 UTC, by a Claude Code agent: the release record's sweep from the signed binaries, and the `publish MCP registry` workflow dispatched with `release=v478.0.0` after the launch gate passed on that release (75 passed, 0 failed, 0 skipped, the maintainer's run at 02:59 UTC) |
| Client | the registry's own publisher, `mcp-publisher` v1.8.7, unmodified, signing in with GitHub's OIDC token for the `io.github.david-engelmann` namespace |
| Result | **655 checks passed, none failed**: "every tool call passed (265 tools in the catalog), the 6 that need a backend included" (the release's own sweep; see `listings/validation/v478.md`) |

## Sent

Not yet, when this record was written. The entry to be published is
`listings/registry/server.json` at the release's commit: `io.github.david-engelmann/maidan`
at 478.0.0, with the remote `https://maidan.world/mcp/streamable` and the
package `ghcr.io/david-engelmann/maidan-server:v478.0.0`. The registry listed
473.0.0 until then.

## Clean install

- Host: a fresh GitHub-hosted runner and the maintainer's Mac (the release record). A clean machine run by another person: not done.
- Steps: `scripts/clean-machine-install.sh` with `MAIDAN_TAG=v478.0.0 STRICT=1`, which pulls the image the entry names.
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

- **The trust sentence in the entry:** the schema has no field for it.
- **A clean install by another person on another machine:** the maintainer's, as in the release record.
- **Re-publishing on each release:** the entry names an image tag, so a new release needs a new publish (`gh workflow run publish-mcp-registry.yml -f release=<tag>`), after the gate passes on it.
