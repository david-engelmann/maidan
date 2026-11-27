# Validation record: the HOL Guard contribution at v478.0.0

This record was written on 2026-11-25, before the contribution is filed. It
covers the files in `listings/hol-guard/` and what they were checked against in
HOL Guard's repository. Nothing was run inside HOL Guard itself, and the
sections below say so where it matters.

| | |
|---|---|
| Listing | HOL Guard's MCP server catalog, five contributions from `listings/hol-guard/` |
| Release | `v478.0.0`, the release the hosted endpoints serve (`/health` reports `v478.0.0` on 2026-11-25) |
| Commit | the head of the pull request that adds this record, on `dev` at `0b2f2410ca7c0029045c9118a741b68d07b45758` |
| Target | `hashgraph-online/hol-guard` at `d1055352dbd7727e61e5b0d19b2e0431e0f8e126` (2026-11-25 16:20:57 UTC, "fix(hooks): let a script grant answer a lone known-vulnerable runtime review (#8562)"), read on 2026-11-25 from a shallow checkout |
| Listed surface | the full catalog on `https://maidan.world/mcp/streamable` (265 tools, 52 named), `/mcp/worker` (23), `/mcp/reviewer` (14), `/mcp/coordinator` (37) and the `maidan` binary over stdio (265 tools, 52 named) |
| Run | 2026-11-25, by a Claude Code agent, reading Guard's files and running `python3 scripts/gen-hol-guard.py --check` and the contribution test below |
| Client | none. Guard was not installed or run |
| Result | **every static check passed (schema, validator rules, collisions, the contribution test). The runtime checks were not run** |

## What Guard accepts, and where it says so

- `CONTRIBUTING.md` lists "MCP server package or tool defaults" as a
  contribution path and sends it to `docs/guard/mcp-server-contributions.md`.
- That guide says community MCP servers ship in the catalog as a reviewed JSON
  file, with an External badge, off until a local administrator turns them on.
  Its "How to contribute" steps are the three files below.
- The catalog held 154 contributions at this commit, most from outside
  projects. Pull requests such as #8159 (a hosted `remote-http` server) and
  #8434 added one contribution, its trust binding and at most a small test.
  Neither carries a sign-off line, and `CONTRIBUTING.md` asks for none. Its
  License section says contributions are licensed under Apache-2.0. The
  `license` field in our files names Maidan's own license (MIT) and is
  metadata, as in the other contributions.
- The two pull request templates in `.github/PULL_REQUEST_TEMPLATE/` are for
  command extensions and cloud action packs. There is none for an MCP server,
  and the merged MCP pull requests above carry a free-form body.

## Which of our files go where

| Ours | In Guard |
|---|---|
| `listings/hol-guard/mcp.maidan.json`, `mcp.maidan-worker.json`, `mcp.maidan-reviewer.json`, `mcp.maidan-coordinator.json`, `mcp.maidan-stdio.json` | `contributions/mcp-servers/` under the same names |
| `listings/hol-guard/trust/command.mcp-maidan*.v1.json` | `contracts/extensions/trust/` under the same names |
| `listings/hol-guard/test_guard_maidan_mcp_contribution.py` | `tests/test_guard_maidan_mcp_contribution.py` |

Guard packages `contributions/mcp-servers/` as a whole directory, so no
packaging edit is needed. Its guide says not to commit the catalog, the native
program or the directory pages, and none is included.

## Checked against Guard's files

- Schema: all five files validate against
  `contracts/mcp-servers/contribution.v1.schema.json` (Draft 2020-12, jsonschema
  4.25.1), run from outside Guard's repository on a copy of that file.
- Validator rules: `guard/runtime/mcp_server_contribution.py` and the native
  lowering in `rust/crates/guard-command/src/` were read, and their rules are
  restated in `scripts/gen-hol-guard.py`, which fails on a breach. They are the
  icon allowlist (`HiMiniGlobeAlt`, `HiMiniCube` and `HiMiniCommandLine` are on
  it), no `allow` for a remote or direct-command server, tool names unique
  after Guard's own folding, at most 80 tool rows, the reserved command list
  (`maidan` is not on it) and the public HTTPS endpoint grammar. The generator
  fails when a rule is broken, which was shown by changing a state to `allow`,
  a command to `node`, an icon and a URL scheme in memory.
- Collisions: no existing contribution uses an id, direct command, server name
  or endpoint of ours, and the string "maidan" appears nowhere else in Guard's
  tree. Guard rejects a duplicate endpoint, alias or command.
- Trust bindings: the five files are byte for byte what
  `refresh_extension_artifacts.py --trust-only` writes (sorted keys, two-space
  indent, `external`), read from that script. The catalog id is the contribution
  id with its first dot turned into a hyphen (`command.mcp-maidan-stdio`).
- Contribution test: `test_guard_maidan_mcp_contribution.py` uses only the
  standard library, pytest and jsonschema, in the style of Guard's
  `tests/test_deed_page_mcp_contribution.py`. It passed (25 tests) against a copy of
  the files laid out as Guard lays them out, and failed when `approval_decide`
  was set to `inherit` in that copy.
- Facts in the files: the hosted endpoints answer `401` to an unauthenticated
  request on 2026-11-25, for all four paths, and
  `/.well-known/oauth-authorization-server` answers `200`. Sign-in is OAuth
  2.0 with Maidan as its own authorization server, and the files say so in
  their descriptions. The server's repository is MIT.
- Tiers: the generator's existing checks passed at this commit. The `read` tier
  is exactly the 123 tools whose annotations say `readOnlyHint`, and every
  write that reaches outside Maidan is reviewed, except `post_message`, whose
  reason `policy.json` records.

## What changed in our files because of the check

- The generator now writes the trust bindings. Guard requires one per
  contribution and CI rejects a pull request without it.
- The generator checks each file against Guard's rules above, so drift from
  Guard's format fails Maidan's `lint` job.
- The descriptions of the four hosted files name the OAuth 2.0 sign-in.
- The README's counts were stale (265 tools, 123 read-only, the worker profile
  at 23) and the coordinator profile was missing from its table.

## Not exercised

- Nothing was installed or run from Guard. Not run: `hol-guard`,
  `validate_mcp_contribution`, `refresh_extension_artifacts.py --check-trust`,
  `tests/test_guard_mcp_server_contribution.py`, the native source compiler and
  Guard's CI. Guard's own pull request checks are the first run of those.
- No call went through a Guard that had a Maidan contribution turned on, so
  the decision a person sees for a reviewed tool, and the endpoint match on a
  real harness, are unverified. The README's "Using it" steps are Guard's
  documented flow, not a run.
- The hosted endpoints were reached only to see the sign-in challenge. No
  tool call was made through Guard against them.
- The `referenceUrls` point at `main` in Maidan's repository. The four files
  they name exist there today, and they show this change's text once it is on
  `main`.

## Sent

- Filed on 2026-11-25 as
  [hashgraph-online/hol-guard#8573](https://github.com/hashgraph-online/hol-guard/pull/8573),
  from the fork `david-engelmann/hol-guard`.
- Approved on 2026-11-26 and merged on 2026-11-27 as `3b4fa3f5`, with every
  check in Guard's CI passing. Their review bots raised no finding that needed
  a change.
