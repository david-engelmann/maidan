# Connected-App Integration Playbook

Lessons, patterns, and expertise from building the Maidan connected-app
E2E harness (2026-10-07). Intended for reuse across all connected-app
pushes — directory submissions, provider integrations, and future
harness work.

## 1. Authentication Tiers

Three tiers, in order of preference. Pick the highest tier the provider
allows; never build a lower tier when a higher one works.

### Tier 1 — API keys (unattended, best)
Environment variables, zero login, fully automatable. Examples:
`XAI_API_KEY`, `GEMINI_API_KEY`, `ANTHROPIC_API_KEY`,
`COPILOT_GITHUB_TOKEN` (fine-grained PAT v2 with Copilot Requests —
classic PATs do not work).

Rules:
- Env var wins; Secure Vault is the fallback; genuine absence is
  `KEY_MISSING` (exit 8), never a traceback.
- Provider account errors (credits exhausted, bad key) surface as
  `CONFIG_ERROR` (exit 11) with the provider's message, sanitized.

### Tier 2 — Email magic-link (automated, no human)
For browser providers without API keys. The harness triggers the
provider's "email me a sign-in" flow, polls Gmail for the sign-in email,
extracts the single LINK, and navigates to it in the same browser
context. Link-only — there is no code-entry UI to drive.

Critical details (learned 2026-10-07):
- The poller must match the REAL email shape, verified against an actual
  email — never guessed. Claude's mail arrives via a Zyla relay
  (`From: zyla@beatgig.com`, NOT `mail.anthropic.com`); the poller
  looking for Anthropic's domain would miss every real email.
- The sign-in link is a Bearer <redacted>: memory-only, never printed,
  logged, persisted, or included in error messages. Playwright embeds
  the URL in `page.goto` errors — wrap navigation and raise sanitized
  errors.
- The Gmail read path must not persist message bodies to disk (use raw
  API `users.messages.get`, never auto-saving shortcuts).
- Strict link-domain validation: exact hostname or real subdomain via
  `urlparse` (`claude.ai.evil.com` must NOT match `claude.ai`).
  Substring matching is a security hole. No "any https" fallback.
- Unverified email shapes are refused unless the operator explicitly
  opts in (`allow_unverified: true` + confirmed `link_domains`).
  Fail closed.
- One self-heal attempt per nightly run, then give up. Never loop.
- Links expire (Claude: 10 minutes). The flow must complete fast;
  `CODE_TIMEOUT` (exit 13) on slow delivery.

### Tier 3 — Manual (browser takeover)
When automation is blocked (e.g., Cloudflare). The user takes over the
live browser, solves challenges and logs in as a human; the session
persists in the browser-task profile for reuse.

Rules:
- Takeover is client-dependent; confirm the client supports it.
- Traffic still originates from the sandbox — a human MAY clear a
  challenge that blocks automation, but it's not guaranteed.
- Never ask for credentials in chat; the user enters them during
  takeover.

## 2. Network Realities

### Sandbox egress
- Egress goes through an explicit HTTP proxy (`https_proxy` env).
  Direct connections are blocked.
- The proxy denies Chromium's direct CONNECT (`policy_denied`) while
  accepting identical bytes via a localhost relay. Workaround: a
  localhost TCP forwarder that sends the CONNECT unfragmented.
- The proxy intercepts TLS with a sandbox CA. curl/Python trust it via
  `SSL_CERT_FILE`; Chromium does not. Workaround: `ignore_https_errors`
  gated STRICTLY on sandbox detection — never elsewhere.

### Cloudflare
- Datacenter egress IPs get hard-blocked (Turnstile "Just a moment"
  never clears, headless or headed, human or not).
- Challenge policy varies by URL: `chatgpt.com/auth/login` blocked,
  `auth.openai.com/log-in` loads clean. Always probe alternate entry
  points (root domain, auth subdomain) before giving up.
- A human via takeover MAY clear what automation cannot — worth one
  attempt, not endless retries.

## 3. Security Invariants

These are non-negotiable; every review checks them:
1. Secrets never touch logs, reports, stderr, or disk (links, keys,
   tokens). Test it: `test_magiclink_no_secret_in_logs`.
2. Link/URL validation is strict hostname matching, never substring.
3. Unverified external shapes fail closed until explicitly confirmed.
4. Config problems are `CONFIG_ERROR` (exit 11), not `TEST_FAILED`.
   A disabled tier is a setup problem, not an assertion failure.
5. No tracebacks to the user for expected failures — clean signals
   with actionable messages.

## 4. Quality Process

### Three reviews before landing
1. Independent red-team re-audit (fresh eyes, checks claims against
   code, runs what can be run).
2. Two-axis line-by-line diff review: (a) Spec fidelity — does the diff
   implement what was asked? (b) Standards — conventions, no
   tautological tests.
3. Own line-by-line review + independent verification (re-run the key
   commands yourself).

### Test discipline
- Every new test must be able to FAIL for a real reason (no tautologies).
- Security invariants get regression tests (evil-domain rejection,
  secret-absence-in-logs, unverified-refusal).
- 48/50+ tests passing locally before any claim of "done".

### Failure signals (exit codes)
3 AUTH_EXPIRED · 4 SETUP_NEEDED · 5 SELECTOR_STALE · 6 SERVER_UNREACHABLE
· 7 TOOL_NOT_CALLED · 8 KEY_MISSING · 9 BINARY_MISSING · 10 MANUAL_ONLY
· 11 CONFIG_ERROR · 13 CODE_TIMEOUT. The registry must mean what it
says — map each failure to the right signal.

## 5. Provider Notes

### Claude (claude.ai)
- Custom connectors: profile → Settings → Customize → Connectors →
  "Add connector" → "Add custom connector". **Verified on Free plan
  2026-10-07** (connectors page fully available, no paywall).
- Dialog step 1: Name (required) + MCP server URL (required, must start
  with `https://` — client-side validated, HTTP rejected). Continue
  triggers an automatic reachability + OAuth-discovery check
  ("Couldn't reach this address" → "Continue anyway" for manual mode).
- Dialog step 2 auth modes: "Sign in now" (OAuth, default), "Sign in
  when needed", **"No sign-in"** (for open-access servers — this is the
  harness's no-auth mode; note there is NO API-key field anywhere).
  OAuth client: Claude's published identity / CIMD (default),
  auto-register / DCR, or own client ID+secret.
- Transport: Streamable HTTP default; SSE legacy auto-selected for
  `/sse` URLs.
- No elicitation/create — approval flows are conversational.
- Login: `https://claude.ai/login`, "Continue with email".
- Sign-in email: subject "Your secure link to Claude.ai is here |
  <timestamp>", via Zyla relay, link on `assets.claude.ai`.
- Selectors rot: re-verify against the live page before each run.

### ChatGPT (chatgpt.com)
- Dev-mode MCP plugin: Settings > Security and login > Developer mode,
  then Plugins page. **Verified 2026-10-07: Developer mode IS available
  on the Free plan** (no paywall on the toggle; the research doc's
  "excludes Free tier" claim was wrong). Plugin creation goes through a
  conversational "Plugin Creator" agent, not a form. Some specific
  connectors (Gmail, Drive, Calendar) show "Upgrade to install" but dev
  mode itself is not gated.
- Login: use `https://auth.openai.com/log-in` (chatgpt.com/auth/login
  is Cloudflare-blocked from some networks).
- OAuth fires lazily on first TOOL use, not at connect.
- No elicitation/create either.
- Widget check: probe for rendered iframes (best-effort).

### MCP integration
- Preflight: `tools/list` before suites; record advertised tools.
- Tool-call correlation via server logs (mark window + skew).
- Sequential `wait_for_tool` calls enforce ORDER via the advancing log
  position — out-of-order calls fail as TOOL_NOT_CALLED.
- Report client surface (web/cli/api/manual) + auth_mode per run.

## 6. When Stuck

1. Check the failure signal — it tells you the category.
2. For Cloudflare: try alternate URLs, then human takeover, then defer.
3. For auth: check the tier — can you go up a tier (API key?)?
4. For selectors: the page changed; re-verify live, don't guess.
5. Document the workaround in this playbook so the next push reuses it.

---
*Built from the Maidan connected-app harness, 2026-10-07. Update this
file when a push teaches something new.*
