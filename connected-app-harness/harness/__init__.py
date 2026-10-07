"""Maidan connected-app E2E harness.

Browser-automation tests for Maidan as a connected app on ChatGPT and
Claude. Every failure mode emits a distinct, greppable signal on stderr;
nobody triaging should ever guess what broke.
"""

__version__ = "0.1.0"

# Failure signals. Each is printed to stderr exactly once, prefixed as
# shown, and paired with a dedicated exit code. Grep for the token.
AUTH_EXPIRED = "AUTH_EXPIRED"          # exit 3: profile session dead; run `harness auth-login`
SETUP_NEEDED = "SETUP_NEEDED"          # exit 4: connector/plugin not installed; run `harness setup`
SELECTOR_STALE = "SELECTOR_STALE"      # exit 5: followed by "<selector> @ <url>"
SERVER_UNREACHABLE = "SERVER_UNREACHABLE"  # exit 6: followed by the base URL
TOOL_NOT_CALLED = "TOOL_NOT_CALLED"    # exit 7: followed by tool name + log excerpt
TEST_FAILED = "TEST_FAILED"            # exit 1: assertion failed, details follow
KEY_MISSING = "KEY_MISSING"            # exit 8: required env var absent; export it, never prompt
BINARY_MISSING = "BINARY_MISSING"      # exit 9: required CLI binary absent, auto-install failed
MANUAL_ONLY = "MANUAL_ONLY"            # exit 10: provider cannot run unattended; see its checklist
CONFIG_ERROR = "CONFIG_ERROR"          # exit 11: server/config problem (not a missing key); details follow
# exit 12 intentionally unassigned (reserved for a future provider-side signal)
CODE_TIMEOUT = "CODE_TIMEOUT"          # exit 13: sign-in email never arrived; check inbox / retry

EXIT_AUTH = 3
EXIT_SETUP = 4
EXIT_SELECTOR = 5
EXIT_SERVER = 6
EXIT_TOOL = 7
EXIT_KEY = 8
EXIT_BINARY = 9
EXIT_MANUAL = 10
EXIT_CONFIG = 11
EXIT_CODE_TIMEOUT = 13
