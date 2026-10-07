# Maidan connected-app E2E harness

E2E tests for Maidan as a connected app across the major AI platforms:
**ChatGPT** (dev-mode MCP plugin) and **Claude** (custom connector) via
browser automation, plus **Grok** (API), **Gemini CLI**, **Copilot CLI**
(MCP + plugin), **Claude Code**, and **Cursor** via CLI/API providers.
**Meta Muse** is manual-only (no automation API exists). Python +
Playwright, standalone — not part of the maidan repo.

**Design rule: the harness never blocks on a human.** David logs in once per
provider; every run after that is unattended. Any auth/session/setup problem
exits fast with a greppable signal instead of waiting.

## Setup

```bash
cd ~/workspace/maidan-connected-app-harness
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
.venv/bin/playwright install chromium
```

Edit `config.yaml`: `server.base_url` (default `http://127.0.0.1:8080` for a
local maidan-server), `server.auth_mode` (`no-auth` for reads, `oauth-stub`
for the write-flow suite), and record the server commit SHA:

```bash
.venv/bin/python -m harness config-sha <sha>
```

To run a local server (from the maidan repo):

```bash
cd ~/workspace/maidan-harness-server
./target/debug/maidan-server >> /tmp/maidan-harness-server.log 2>&1
```

The harness tails that log for server-side assertions (the strongest
signal: UI text is secondary, log evidence doesn't break when selectors do).

## Providers

| Provider | Mechanism | `test` | Notes |
|---|---|---|---|
| `claude`, `chatgpt` | browser | yes | headed `auth-login` once; then unattended |
| `grok` | xAI Responses API, MCP in `tools[]` | yes | needs `XAI_API_KEY` + public MCP URL (loopback refused) |
| `gemini` | `gemini` CLI + linked extension | yes | binary auto-installed via npm; `extensions link` with consent |
| `copilot-mcp` | `copilot mcp add` | yes | binary auto-installed via npm |
| `copilot-plugin` | plugin dir-source marketplace | yes | separate mechanism from MCP (research §1.5) |
| `claude-code` | `claude mcp add` + `claude -p` | yes | **only provider with elicitation tests** |
| `cursor` | `~/.cursor`/project `mcp.json` | no | manual bootstrap; see `checklists/cursor.md` |
| `meta` | conversational walkthrough | no | no automation API; see `checklists/meta-muse.md` |

Nightly runs: claude, chatgpt, grok, gemini, copilot-mcp, copilot-plugin,
claude-code. Cursor and Meta are excluded (manual-only).

API keys and CLI binaries are **environment, not code**. Export before
running; the harness fails fast with `KEY_MISSING` (exit 8) when one is
absent and never prompts:

| Provider | Env var(s) | How to get it |
|---|---|---|
| `grok` | `XAI_API_KEY` | xAI console → API keys |
| `gemini` | `GEMINI_API_KEY` | Google AI Studio (free tier works) |
| `claude-code` | `ANTHROPIC_API_KEY` | Anthropic console → API keys |
| `copilot-mcp`, `copilot-plugin` | `COPILOT_GITHUB_TOKEN` (or `GH_TOKEN`/`GITHUB_TOKEN`) | fine-grained PAT with the **Copilot Requests** permission; classic PATs do not work |

The CLIs self-install via npm when missing (`BINARY_MISSING` exit 9 if
that fails). Never hardcode keys, never commit them. Grok needs a *public*
MCP URL (the xAI API cannot reach localhost); set `server.base_url` to the
public URL for Grok runs.

## The one human checkpoint

Browser providers: run once per provider, on a machine with a display:

```bash
.venv/bin/python -m harness auth-login claude
.venv/bin/python -m harness auth-login chatgpt
```

A headed browser opens; log in manually (including 2FA/SSO), wait for the
chat UI, press Enter. The profile is saved under `profiles/`.

### Magic-link self-healing (Tier 2 auth)

The test accounts live under **zyla@beatgig.com**. When a browser
provider's session expires, the harness re-logs-in headlessly instead of
asking David again: it submits the address on the provider's login page,
polls Gmail for the sign-in email, opens the single sign-in LINK it
contains in the same browser context, and re-runs the two-marker auth
verdict. Link-based by design (no code-entry UI to drive).

**One-time human steps (David):**
1. Create a Claude account and a ChatGPT account using
   `zyla@beatgig.com` (sign up with email).
2. Link the `zyla@beatgig.com` Gmail account to the Gmail connector
   (the currently connected `david.engelmann44@gmail.com` cannot see its
   mail). Find its account id with `hatch_gws_cli gmail accounts` and,
   if it is not the default account, set it as `account:` below.
3. Enable the tier in `config.yaml`:

```yaml
providers:
  claude:
    magic_link:
      enabled: true
      email: "zyla@beatgig.com"
      account: null   # or the zyla@beatgig.com account_id
```

**How it works:** `harness auth-login claude --via-email` (or plain
`auth-login` once `enabled: true` — it becomes the default) opens the
login page headlessly, submits the address, polls Gmail for the
provider's sign-in email (Claude: the "Your secure link to Claude.ai
is here" link, opened in the same browser context; ChatGPT: a sign-in
link if the account offers one — standard login is email+password,
and the flow fails fast with guidance if a password field appears
instead), and re-runs the two-marker auth verdict. The
nightly does this automatically: on `AUTH_EXPIRED` it attempts exactly
one magic-link re-login and one re-check, records both in the report,
and never loops.

**Security:** the sign-in link is a bearer credential — memory only,
never printed, logged, or persisted (enforced by
`test_magiclink_no_secret_in_logs`). The poller only reads Gmail (tight
`to:`+`from:`+recency queries); it creates, labels, forwards, or
deletes nothing. Tip: add a Gmail filter filing mail to
`zyla@beatgig.com` under a label to keep the inbox tidy (the harness
never creates filters itself).

**Caveats:** Claude's link flow is verified against a real email
(2026-10-07). ChatGPT's sign-in-link email is UNVERIFIED, so the tier
refuses to run for ChatGPT until you confirm the real email shape and
opt in explicitly:

```yaml
providers:
  chatgpt:
    magic_link:
      enabled: true
      email: "zyla@beatgig.com"
      allow_unverified: true   # you confirmed the real email shape
      link_domains: ["auth.openai.com"]  # confirmed sign-in link domains
```

Without both, `poll_for_signin` raises `CONFIG_ERROR` rather than act
on a guessed shape. Extracted links must also pass a strict hostname
allowlist (exact host or real subdomain — `claude.ai.evil.com` does
not match `claude.ai`); anything else keeps polling instead of
navigating somewhere untrusted. If the login page shows a password
field instead, the flow fails fast with guidance to use headed
`auth-login` once. If the sign-in email never arrives within
120s,
the run fails with `CODE_TIMEOUT` (exit 13).

CLI providers have no login step: `harness check-auth <provider>` verifies
env vars and binaries. Cursor and Meta need a human every time (see
checklists); the harness refuses to pretend otherwise (`MANUAL_ONLY`, exit
10).

## Commands

```bash
# Fail fast unless the saved session is alive (exit 3 + AUTH_EXPIRED if not)
.venv/bin/python -m harness check-auth claude

# Install the connector/plugin if missing (idempotent; run rarely)
.venv/bin/python -m harness setup claude

# Run tests: new chat -> prompt -> server-side assertions
# (CLI providers run prompts via their own binary instead of a browser)
.venv/bin/python -m harness test claude --suite reads
.venv/bin/python -m harness test chatgpt --suite all
.venv/bin/python -m harness test grok --suite reads
.venv/bin/python -m harness test claude-code --suite elicitation

# Nightly: check-auth + setup-verify + all suites, all providers,
# timestamped JSON report in reports/. Cron-friendly, zero human in loop.
.venv/bin/python -m harness nightly

# Review battery: unit tests + all failure signals, no credentials needed.
# (AUTH_EXPIRED is exercised separately via check-auth with no profile.)
.venv/bin/python -m harness selftest
```

## Failure signals (stderr, greppable, each with its own exit code)

| Signal | Exit | Meaning |
|---|---|---|
| `AUTH_EXPIRED` | 3 | Session dead, bot interstitial, or 25s without two logged-in markers. Run `harness auth-login <provider>` once. |
| `SETUP_NEEDED` | 4 | Connector/plugin missing or URL mismatch. Run `harness setup <provider>`. |
| `SELECTOR_STALE: <key> tried [...] @ <url> :: landmark: ...` | 5 | Provider UI changed; the landmark shows what the page actually contained. Repair the selector registry. |
| `SERVER_UNREACHABLE <url>` | 6 | Maidan server not answering. |
| `TOOL_NOT_CALLED <tool> :: <log>` | 7 | Prompt didn't trigger the tool within the timestamp window; log excerpt attached. |
| `TEST_FAILED` | 1 | Assertion failed; details follow. |
| `KEY_MISSING` | 8 | Required API key env var absent. Export it and retry — the harness never prompts. |
| `BINARY_MISSING` | 9 | CLI binary missing and auto-install failed. Install instructions are printed; retry after installing. |
| `MANUAL_ONLY` | 10 | Provider cannot run unattended (cursor, meta). See `checklists/`. |
| `CONFIG_ERROR` | 11 | Server/config problem that is not a missing key (e.g. Grok's MCP URL is loopback and api.x.ai can't reach it). Remediation is printed. |
| `CODE_TIMEOUT` | 13 | Magic-link sign-in email never arrived within 120s. Check the inbox / retry. |

Auth verdicts require **two independent markers** (composer + account menu);
one marker alone is `unknown` → `AUTH_EXPIRED`, never a false logged-in.
Marker results (seen/missing) are logged in every report.

Tool-call assertions are **timestamp-correlated**: only log entries at or
after the prompt's mark time (minus 5s skew allowance) count. A tool call
from before the prompt can never satisfy the assertion.

## Layout

- `harness/` — CLI, auth, browser profiles, server log probe, providers, reports
- `checklists/` — L3b 5-minute human checklists (ChatGPT dev-mode, claude.ai
  connectors) for the browser-only flows, executable by a stranger
- `tests/` — unit tests for non-browser logic (`config`, `probe`, signals)
- `profiles/` — per-provider persistent browser profiles (gitignored)
- `reports/` — timestamped nightly/test reports (gitignored)

## What the automated suites cover

Per provider: `reads` (tool calling), `multi_tool` (agentic sequence:
list → read → post), `approval` (conversational approval + Maidan approval
tool), `writes` (write-flow auth via the dev OAuth stub — runs ONLY when
`server.auth_mode: oauth-stub`, otherwise skipped; never conflated with
the no-auth read path).

`elicitation` (approval flow via `elicitation/create`) runs **only on
Claude Code** — the only in-scope provider implementing it (research §5.2).
It additionally requires `elicitation.enabled: true` in config.yaml
(default false); everywhere else it is skipped, never faked. The
`approval` suite on other providers uses conversational approval precisely
because neither implements `elicitation/create`.

ChatGPT additionally probes for MCP Apps widget rendering (best-effort:
the server must emit a widget payload).

`setup` (install) is split from `test` (exercise) on purpose: install runs
rarely (only when the dev server URL changes); `test` starts with the cheap
`setup_probe` (connector listed + URL matches config, no chat round-trip)
and exits `SETUP_NEEDED` instead of burning a test on a missing connector.
