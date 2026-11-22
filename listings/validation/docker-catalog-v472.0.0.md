# Validation record: the Docker MCP Catalog at v472.0.0

This record was written on 2026-11-22, after the entry was sent. The template
asks for it before. The release it rests on had its release-level record
first (`listings/validation/v472.md`), and what this listing's own record adds
is below, with what was not done named under "Still open".

| | |
|---|---|
| Listing | the Docker MCP Catalog (Docker Desktop's MCP Toolkit), `listings/docker/remote/`, sent as `servers/maidan/` in `docker/mcp-registry` |
| Release | `v472.0.0` |
| Commit | `a9be563061ffb95648570feb172aba0d92ab1869` |
| Server image | `ghcr.io/david-engelmann/maidan-server:v472.0.0`, index digest `sha256:311762ea2a5a9b98529cacf533093a6220538e7a097203dcb25b7af8952f5309`, cosign signature verified against `release.yml@refs/tags/v472.0.0`. Both `linux/amd64` and `linux/arm64` carry `io.modelcontextprotocol.server.name=io.github.david-engelmann/maidan` |
| Binaries | `maidan-aarch64-apple-darwin.tar.gz`, sha256 `3cc101b6c1c2abdc50bd2891c455c7562947c0f0fe6f6d9a54fb594cab53a49c`, cosign bundle verified against `release.yml@refs/tags/v472.0.0`. `SHA256SUMS` (sha256 `e3b1bb49deba00898dd1cd52b535e73414acbe70a6fed0d2869e0ff4999c8f85`) is cosign-verified the same way, and the tarball matches its line. Extracted: `maidan-server` sha256 `980356f7ffbf50ac961978fd5de67a826e25c29187efe7242ac03854d13a7361`, `maidan` sha256 `6c93bc53e3213e35734c68626e0f2c84877da4b8163faaab6d0252a2d47f5dd4` (`maidan --version` prints `maidan v472.0.0`) |
| Listed surface | the full MCP catalog, 265 tools, read live from the hosted endpoint `https://maidan.world/mcp/streamable` (the entry's `tools.json` is empty, as for every remote server there) |
| Run | 2026-11-21 UTC, by a Claude Code agent: the release record's sweep from the signed binaries, and the catalog's own `go run ./cmd/validate --name maidan` at the catalog's `main` that day, which passed every check (name, directory, title, YAML, secrets, icon, dynamic tools, remote, OAuth dynamic configuration) |
| Client | the catalog's validator, unmodified. Docker Desktop's MCP Toolkit itself was not run against the entry: the entry is not in a released catalog until Docker merges it |
| Result | **655 checks passed, none failed**: "every tool call passed (265 tools in the catalog), the 6 that need a backend included" (the release's own sweep; see `listings/validation/v472.md`) |

## Sent

Filed on 2026-11-21 as docker/mcp-registry#7499, after the launch gate passed on `v472.0.0` and the maintainer cleared the name and logo. It waits in Docker's queue. The entry names no release, so `v473.0.0` and later need no change to it.

## Clean install

- Host: not applicable to a remote entry, which installs nothing. The hosted endpoint was probed from outside on the release (the release record's "maidan.world" section).
- Steps: in the Toolkit, enable the server and run `docker mcp oauth authorize maidan`. Not run, for the reason above.
- Result: not run.

## Tools in the listed surface

- Each tool succeeds with valid parameters: pass, for all 265 (the release
  record's sweep).
- Each tool returns an actionable error with bad parameters, naming the
  argument it got wrong: pass, for all 265.
- Every tool carries `readOnlyHint`, `destructiveHint`, `openWorldHint` and a
  title (the annotation contract test passed at this commit): pass.

## Example prompts through the platform's own model

Not run. The Toolkit hands the tools to whichever model the person connects; there is no Docker model to run them through.

## Identity and tokens

- Token the listing authenticates as: none is shipped. A person authorizes with OAuth (dynamic client registration and PKCE); maidan.world accepts the Toolkit's callback `https://mcp.docker.com/oauth/callback` since `v470.0.0`, checked by registering a client with it.
- Reviewer token: none. Sign-up at maidan.world is open, so a reviewer makes an account; the pull request says so.
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

- Where it appears in the listing: `listings/docker/remote/readme.md` links the docs; the sentence itself is in `docs/Listing.md` and not in the three files the catalog takes.

## Still open before this listing ships

- **This record before the filing:** it was written after.
- **The OAuth sign-in from Docker Desktop itself:** not run end to end. The callback's registration is checked; the Toolkit's own flow can be run only once the entry is in a catalog build, or against a local catalog.
- **Docker's review:** open, and their queue is slow. A reviewer's comments on the pull request are answered there.
- **The trust sentence in the entry's readme:** not there yet; it can go in with the first change a reviewer asks for.
