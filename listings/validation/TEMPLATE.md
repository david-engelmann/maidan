# Validation record: <listing> at <release>

Copy this file to `listings/validation/<listing>-<release>.md` (for example
`cursor-v420.0.0.md`) and fill in every field before the listing is published
or submitted (Open Work Next 14). A field that cannot be filled says why and
what blocks it under **Still open**. Any change to the listed surface, and
every new release, re-runs the record before a resubmission. Whoever cuts the
release owns the re-run.

| | |
|---|---|
| Listing | <directory or catalog, and the package path in this repo> |
| Release | `<vX.Y.Z>` |
| Commit | `<full 40-character SHA>` |
| Server image | `<image>:<tag>`, index digest `sha256:<digest>` |
| Binaries | `<artifact>`, sha256 `<hash>`, cosign bundle verified against `release.yml@refs/tags/<vX.Y.Z>` |
| Listed surface | <the full tool set, or the curated profile and its tool count> |
| Run | <date>, by <who>, <script or steps>, against <instance> |
| Client | <the platform's client and version, unmodified> |
| Result | **<N> of <M> checks passed** |

## Clean install

- Host: <a clean machine other than the maintainer's>, <OS and version>
- Steps: <the listing's published install steps, as a user follows them>
- Result: <installed and connected, or what failed>

## Tools in the listed surface

- Each tool succeeds with valid parameters: <pass, or the failing tools>
- Each tool returns an actionable error with bad parameters, naming the
  argument it got wrong: <pass, or the failing tools>
- Every tool carries `readOnlyHint`, `destructiveHint`, `openWorldHint` and a
  title (the annotation contract test passed at this commit): <pass or fail>

## Example prompts through the platform's own model

| Prompt | Tools expected | Tools picked | Pass |
|---|---|---|---|
| <prompt from the listing materials> | <tools> | <tools> | <yes/no> |

## Identity and tokens

- Token the listing authenticates as: <capability profile, prefix, expiry>
- Reviewer token: <expiry>, outlasts the longest expected review wait of
  <duration>: <yes/no>
- Never a `token:admin` or non-expiring token: <confirmed>

## Protocol-layer properties

- No surface lets the supervised agent compose, trigger or satisfy its own
  approval: <how checked>
- Every approval binds to a verified human identity, per-user credentials at
  minimum: <how checked>
- The close gate stays server-side and durable: <how checked>

## Trust model

The listing's materials and tests carry this sentence: Provider-rendered
approval UI is untrusted display, the Maidan console is the trust root, and an
approval whose evidence the human did not see in the console is provisional.

- Where it appears in the listing: <link or path>

## Still open before this listing ships

- <item>: <why, and what blocks it>
