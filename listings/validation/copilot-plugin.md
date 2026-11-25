# Validation record: the GitHub Copilot CLI plugin package at v478.0.0

This record was written on 2026-11-25. It covers the package as it stands on
`dev` after `v478.0.0`: the root `plugin.json`, the `skills/` directory and
`.github/plugin/marketplace.json`. The tool surface the skills name is the one
the release-level record validated (`listings/validation/v478.md`). What this
record adds is the package's shape checked against GitHub's published plugin
documentation, and the skills checked against the current catalog and
profiles. The `copilot` CLI is not installed on the machine that wrote this
record, so no install was run. That is named under "Not exercised".

| | |
|---|---|
| Listing | the GitHub Copilot CLI plugin marketplace, `.github/plugin/marketplace.json`, and the plugin at the repository root (`plugin.json`, `skills/`) |
| Release | `v478.0.0` |
| Commit | `3d3b346758113b823f59d0dad23f0a49b77527f7`, the release commit whose tool surface the skills name. The package was rebased onto `dev` at `0b2f2410ca7c0029045c9118a741b68d07b45758` |
| Server image | `ghcr.io/david-engelmann/maidan-server:v478.0.0`, index digest `sha256:b0e45c79a640a03dd89da86884b65f66f8cef485010828d3e8cb225f7152abf5` (the release record's) |
| Binaries | the release record's (`listings/validation/v478.md`). No binary was run for this listing |
| Listed surface | the full MCP catalog, 265 tools, on `/mcp/streamable`. The worker profile (23 tools) on `/mcp/worker` and the reviewer profile (14 tools) on `/mcp/reviewer` serve the subsets named below |
| Run | 2026-11-25, by a Claude Code agent, reading GitHub's documentation and running the repository's non-cargo checks. No instance was run |
| Client | none. The `copilot` CLI was not available |
| Result | the documentation checks and the repository checks below passed. The install and every Copilot-side behavior were not exercised |

## What a user types

```sh
copilot plugin marketplace add david-engelmann/maidan
copilot plugin install maidan@maidan-plugins
copilot mcp add --transport http maidan https://YOUR-INSTANCE/mcp/streamable \
  --header "Authorization: Bearer $MAIDAN_TOKEN"
```

The first command registers the marketplace under the `name` in
`marketplace.json`, `maidan-plugins`. The second installs the plugin `maidan`
from it. The third adds the user's own instance as an MCP server, because the
plugin carries none (see below). All three command forms come from GitHub's
documentation.

## Checked against GitHub's documentation

Pages read on 2026-11-25:
- Plugin and marketplace reference,
  `docs.github.com/en/copilot/reference/copilot-cli-reference/cli-plugin-reference`
- Creating a plugin marketplace,
  `docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/plugins-marketplace`
- Finding and installing plugins,
  `docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/plugins-finding-installing`
- CLI command reference,
  `docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference`

What each page says, and what the package does:
- **Marketplace file path.** The catalog is `marketplace.json` in the
  repository's `.github/plugin/` directory. The package has it there.
- **Marketplace fields.** `name` (kebab-case, at most 64 characters), `owner`
  (an object with `name`) and `plugins` are required. Each plugin entry needs
  `name` and `source`, and `description` is capped at 1024 characters. The
  package sets all of them, and its descriptions are under the cap.
- **Plugin source.** A relative path string, resolved from the repository root,
  with the leading `./` optional. The package's entry uses `"."`, the
  repository root, because the plugin lives there. The pages document no
  example of a root-level source.
- **Manifest format.** A `$schema` of
  `https://agent-plugins.org/schemas/1.0.0/plugin.schema.json` (or 1.1.0) opts
  a plugin into Agent Plugins semantics, and its manifest sits at the plugin
  root. The package's `plugin.json` has that `$schema` and sits at the root.
- **Manifest fields.** In that format the allowed fields are `$schema`, `name`,
  `version`, `description`, `author`, `homepage`, `repository`, `license`,
  `keywords` and `extensions`. Component path fields (`skills`, `agents`,
  `hooks`, `mcpServers`, `lspServers`) are not allowed. The package uses only
  allowed fields, and `copilot_plugin_manifest` asserts that none of the
  component fields is present.
- **Skills.** In the Agent Plugins format, skills load only from the plugin's
  `skills/` directory, with no root `SKILL.md` fallback. The package's skills
  are `skills/maidan-claimant/` and `skills/maidan-reviewer/`, each with a
  `SKILL.md`.
- **MCP servers.** The Agent Plugins format declares none in the manifest. The
  package therefore carries no server, host or token, and the user adds the
  server with `copilot mcp add`. The command reference documents
  `copilot mcp add --transport http SERVER-NAME URL` with a repeatable
  `--header "HEADER: VALUE"` option.
- **Review.** The marketplace pages describe a repository with a catalog file
  and no submission or review step. Access needs read access to the repository
  and a GitHub sign-in in the client.

**Which branch `marketplace add` reads.** The pages do not say. The command
reference lists `add SOURCE` as `owner/repo`, `owner/repo#ref`, a URL or a
local path, and does not say how `#ref` resolves or what a bare `owner/repo`
reads. This repository's default branch is `dev`, and releases fast-forward
`main`. A user who wants the released tree types
`copilot plugin marketplace add david-engelmann/maidan#main`. What the bare
form reads was not exercised.

## The skills against the current catalog and profiles

- `skills/maidan-claimant` and `skills/maidan-reviewer` are byte for byte the
  skills under `examples/claude-code/` (`diff -r` printed nothing). They were
  refreshed from the examples on 2026-11-25, because the copies from 2026-11-01
  had drifted. The old claimant lacked `release_own_claims`, the
  `maidan.change.result/1` envelope name (it said `pi.change.result/1`),
  `add_evidence_record` and `list_evidence_records`, and the decline reasons
  on `release_claim`. The old reviewer lacked the review seat's pull (`claim_next_workspace_thread` with `kind: "review"`), the own-family decline and the current close and advisor rules.
- `python3 scripts/check-agent-skills.py` passed: 12 skills cover 265 of 265
  tools in the catalog, and the two bundled skills name no tool that
  `crates/maidan-mcp/src/tools/catalog.json` lacks. All 265 names are the ones
  in `contracts/mcp-tool-names.json`.
- The profile endpoints `/mcp/worker`, `/mcp/reviewer` and `/mcp/coordinator`
  exist in `crates/maidan-mcp/src/profiles.rs` and `crates/maidan-server/src/app.rs`.
  The worker profile has 23 tools and the reviewer profile 14.
- The skills name tools the profiles do not serve. The claimant's Tools list
  names 34 tools and 14 are outside the worker profile, among them
  `add_evidence_record`, `list_evidence_records`, `claim_thread`,
  `register_thread_pull`, `upload_artifact` and the multipart upload tools.
  The reviewer's Tools list names 32 and 19 are outside the reviewer profile,
  among them the land-gate, vote and `claim_next_workspace_thread` tools. The
  install command therefore points at `/mcp/streamable`. A seat that runs only
  the core worker loop can use `/mcp/worker`. The bundled reviewer skill needs
  the full endpoint.
- The claimant runbook's harness table gained a row for Copilot CLI
  (`"harness": "copilot"`), in the example and in the copy.

## Repository checks

Run from the repository root on 2026-11-25:
- `bash scripts/check-no-internal-names.sh` passed.
- `bash scripts/check-our-domains.sh` passed.
- `bash scripts/changelog-fragments.sh check` passed with the package's
  fragment, `changelog.d/next10-copilot-plugin.md`.
- `python3 scripts/gen-integration-main.py` registered
  `crates/maidan-server/tests/copilot_plugin_manifest.rs` in
  `crates/maidan-server/tests/it/main.rs`.
- `plugin.json` and `.github/plugin/marketplace.json` parse as JSON.

`copilot_plugin_manifest` was not run on this machine. It runs in CI's
integration job. It asserts the manifest's `$schema` and name, that the
description says the plugin is free and production is self-hosted, that no
component field or `mcp.json` is present, that the bundled skills equal the
examples, and that the marketplace lists the one root plugin.

## Clean install

- Host: none. No machine ran the install.
- Steps: the three commands under "What a user types".
- Result: not run. See "Not exercised".

## Tools in the listed surface

- Each tool succeeds with valid parameters: pass, for all 265 (the release
  record's sweep, `listings/validation/v478.md`). Not re-run for this listing.
- Each tool returns an actionable error with bad parameters, naming the
  argument it got wrong: pass, for all 265 (the same sweep).
- Every tool carries `readOnlyHint`, `destructiveHint`, `openWorldHint` and a
  title: the annotation contract test passed on `v478.0.0`.

## Example prompts through the platform's own model

Not run. The prompts for this client are the `copilot-cli` entry in
`listings/prompts.json`. Running them needs the `copilot` CLI connected to an
instance.

## Identity and tokens

- Token the listing authenticates as: one the user mints for the seat and
  passes to `copilot mcp add`. The package holds none.
- Reviewer token: the user's. The package sets no expiry.
- Never a `token:admin` or non-expiring token: the package and its text carry
  no token, and `copilot_plugin_manifest` refuses a `Bearer` string or
  `maidan.world` in the manifest.

## Protocol-layer properties

- No surface lets the supervised agent compose, trigger or satisfy its own
  approval: the package adds no server-side surface. It carries two skills and
  two manifests. The server's gates are those the release record checked.
- Every approval binds to a verified human identity: as in the release record.
- The close gate stays server-side and durable: as in the release record.

## Trust model

Provider-rendered approval UI is untrusted display, the Maidan console is the
trust root, and an approval whose evidence the human did not see in the console
is provisional.

- Where it appears in the listing: `docs/Listing.md`, "The Copilot CLI
  package". The marketplace and plugin manifests have no field for it.

## Not exercised

- **No install was run.** The `copilot` CLI is not installed on the machine
  that wrote this record. `copilot plugin marketplace add`,
  `copilot plugin install` and `copilot mcp add` were not run against this
  repository or an instance.
- **The branch a bare `owner/repo` reads** is not stated in the documentation
  and was not tested.
- **A root-level `"source": "."`** is not shown in the documentation's
  examples and was not loaded by the CLI.
- **The skills inside Copilot CLI.** That Copilot discovers both skills and
  runs the claimant's `change_result.py` was not observed.
- **The example prompts** through Copilot's own model.
- **A clean install by another person on another machine.**

## Still open before this listing ships

- **A run of the three commands** on a machine with the `copilot` CLI, against
  an instance on the release, with the result added here.
- **Re-running this record on each release.** A change to the skills, the
  profiles or the tool catalog changes what the package names.
