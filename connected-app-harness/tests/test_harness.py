"""Unit tests for non-browser harness logic."""
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from harness import (
    AUTH_EXPIRED, SETUP_NEEDED, SELECTOR_STALE, SERVER_UNREACHABLE,
    TOOL_NOT_CALLED, TEST_FAILED,
    EXIT_AUTH, EXIT_SETUP, EXIT_SELECTOR, EXIT_SERVER, EXIT_TOOL,
)
from harness import config as config_mod
from harness.probe import ServerProbe, ProbeError
from harness.providers import get_provider
from harness.reporting import Report


def test_signals_distinct():
    signals = [AUTH_EXPIRED, SETUP_NEEDED, SELECTOR_STALE,
               SERVER_UNREACHABLE, TOOL_NOT_CALLED, TEST_FAILED]
    assert len(set(signals)) == len(signals), "signals must be distinct"


def test_exit_codes_distinct():
    codes = [EXIT_AUTH, EXIT_SETUP, EXIT_SELECTOR, EXIT_SERVER, EXIT_TOOL]
    assert len(set(codes)) == len(codes)


def test_config_loads():
    cfg = config_mod.load()
    assert cfg["server"]["base_url"].startswith("http")
    assert cfg["server"]["mcp_path"] == "/mcp"
    assert "claude" in cfg["providers"] and "chatgpt" in cfg["providers"]
    assert "reads" in cfg["suites"]


def test_mcp_url():
    cfg = config_mod.load()
    assert config_mod.mcp_url(cfg) == "http://127.0.0.1:8080/mcp"


def test_providers_known():
    cfg = config_mod.load()
    assert get_provider("claude", cfg).name == "claude"
    assert get_provider("chatgpt", cfg).name == "chatgpt"
    try:
        get_provider("nonexistent-provider", cfg)
    except SystemExit:
        pass
    else:
        raise AssertionError("unknown provider should SystemExit")


def test_probe_requires_log_file():
    cfg = config_mod.load()
    cfg["server"]["log_file"] = ""
    probe = ServerProbe(cfg)
    try:
        probe.wait_for_tool("search", timeout=0.1)
    except ProbeError as e:
        assert e.signal == TOOL_NOT_CALLED
        assert e.exit_code == EXIT_TOOL
    else:
        raise AssertionError("probe without log_file must raise, never pass")


def test_probe_finds_tool_call():
    with tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False) as f:
        f.write('{"jsonrpc":"2.0","method":"tools/call",'
                '"params":{"name":"search","arguments":{"q":"onboarding"}}}\n')
        path = f.name
    cfg = config_mod.load()
    cfg["server"]["log_file"] = path
    probe = ServerProbe(cfg)
    probe._pos = 0  # read from start for the test
    m = probe.wait_for_tool("search", timeout=2.0)
    assert m is not None
    os.unlink(path)


def test_probe_enforces_tool_order():
    """Out-of-order tool calls fail: post_message before search must NOT
    satisfy a later wait_for_tool('post_message')."""
    import tempfile, os
    from harness.probe import ServerProbe, ProbeError
    with tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False) as f:
        # post_message FIRST (wrong order), then search.
        f.write('{"jsonrpc":"2.0","method":"tools/call",'
                '"params":{"name":"post_message","arguments":{}}}\n')
        f.write('{"jsonrpc":"2.0","method":"tools/call",'
                '"params":{"name":"search","arguments":{}}}\n')
        path = f.name
    cfg = config_mod.load()
    cfg["server"]["log_file"] = path
    probe = ServerProbe(cfg)
    probe._pos = 0
    probe._mark_time = 0
    # Finds search (skipping the early post_message)...
    m = probe.wait_for_tool("search", timeout=2.0, settle_s=0)
    assert m.tool == "search"
    # ...but the early post_message is behind the position: not found.
    try:
        probe.wait_for_tool("post_message", timeout=1.0, settle_s=0)
    except ProbeError as e:
        assert "TOOL_NOT_CALLED" in str(e.signal)
        os.unlink(path)
        return
    os.unlink(path)
    raise AssertionError("out-of-order post_message should not match")


def test_probe_finds_in_order_tool_calls():
    """M1 regression: in-order search -> post_message must BOTH be found.
    _pos must advance past the match, not to EOF."""
    import tempfile, os
    from harness.probe import ServerProbe
    with tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False) as f:
        f.write('{"jsonrpc":"2.0","method":"tools/call",'
                '"params":{"name":"search","arguments":{}}}\n')
        f.write('{"jsonrpc":"2.0","method":"tools/call",'
                '"params":{"name":"post_message","arguments":{}}}\n')
        path = f.name
    cfg = config_mod.load()
    cfg["server"]["log_file"] = path
    probe = ServerProbe(cfg)
    probe._pos = 0
    probe._mark_time = 0
    m1 = probe.wait_for_tool("search", timeout=2.0, settle_s=0)
    assert m1.tool == "search"
    m2 = probe.wait_for_tool("post_message", timeout=2.0, settle_s=0)
    assert m2.tool == "post_message"
    os.unlink(path)


def test_all_providers_define_surface():
    """M2 regression: every provider must define surface (web/cli/api/manual)
    — the client_info telemetry reads it unconditionally."""
    import harness.providers as providers_mod
    from harness.providers.base import Provider
    from harness.providers.cli_base import CLIProvider
    import inspect
    seen = set()
    for name, cls in inspect.getmembers(providers_mod, inspect.isclass):
        if not name.endswith("Provider"):
            continue
        if cls in (Provider, CLIProvider):
            continue
        # Only concrete providers with a name attribute set.
        if not getattr(cls, "name", None) or cls.name in ("base", "cli"):
            continue
        assert isinstance(getattr(cls, "surface", None), str), (
            f"{name} has no surface attribute")
        assert cls.surface in ("web", "cli", "api", "manual"), (
            f"{name}.surface={cls.surface!r} not a known surface")
        seen.add(cls.name)
    # The provider that crashed: copilot-plugin must be covered.
    assert "copilot-plugin" in seen, f"copilot-plugin not checked; saw {seen}"


def test_probe_preflight_malformed_is_clean_error():
    """M3 regression: malformed tools/list shapes -> ProbeError, never a
    raw AttributeError traceback."""
    from harness.probe import ServerProbe, ProbeError
    import urllib.request
    cfg = config_mod.load()
    cfg["server"]["base_url"] = "http://127.0.0.1:1"
    probe = ServerProbe(cfg)

    class FakeResp:
        def __init__(self, body): self._body = body
        def __enter__(self): return self
        def __exit__(self, *a): return False
        def read(self): return self._body

    orig = urllib.request.urlopen
    for bad in (
        b'{"jsonrpc":"2.0","id":1,"result":"oops"}\n',
        b'{"jsonrpc":"2.0","id":1,"result":{"tools":["notadict"]}}\n',
        b'{"jsonrpc":"2.0","id":1,"result":{"tools":"nope"}}\n',
    ):
        urllib.request.urlopen = lambda req, timeout=15, b=bad: FakeResp(b)
        try:
            probe.preflight()
        except ProbeError as e:
            assert "SERVER_UNREACHABLE" in str(e.signal)
        else:
            raise AssertionError(f"preflight should fail cleanly for {bad!r}")
        finally:
            urllib.request.urlopen = orig


def test_probe_preflight_parses_tools_list():
    """Preflight extracts tool names from a tools/list response."""
    import tempfile, os, json
    from harness.probe import ServerProbe
    cfg = config_mod.load()
    cfg["server"]["base_url"] = "http://127.0.0.1:1"  # unused (mocked)
    probe = ServerProbe(cfg)
    import urllib.request
    import harness.probe as probe_mod

    class FakeResp:
        def __enter__(self): return self
        def __exit__(self, *a): return False
        def read(self):
            return (b'data: {"jsonrpc":"2.0","id":"harness-preflight",'
                    b'"result":{"tools":[{"name":"search"},{"name":"post_message"}]}}\n')

    orig = urllib.request.urlopen
    urllib.request.urlopen = lambda req, timeout=15: FakeResp()
    try:
        out = probe.preflight()
    finally:
        urllib.request.urlopen = orig
    assert out["tools"] == ["search", "post_message"]
    assert out["count"] == 2


def test_probe_timeout_carries_excerpt():
    with tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False) as f:
        f.write("unrelated line one\nunrelated line two\n")
        path = f.name
    cfg = config_mod.load()
    cfg["server"]["log_file"] = path
    probe = ServerProbe(cfg)
    probe._pos = 0
    try:
        probe.wait_for_tool("post_message", timeout=0.5)
    except ProbeError as e:
        assert e.signal == TOOL_NOT_CALLED
        assert "unrelated line two" in e.detail
    else:
        raise AssertionError("expected TOOL_NOT_CALLED")
    os.unlink(path)


def test_report_summary():
    cfg = config_mod.load()
    r = Report(cfg, "unit-test")
    r.command(["test", "claude"])
    r.tool_called("claude", "search", "reads")
    r.suite_result("claude", "reads", True, "response 42 chars")
    r.suite_result("claude", "approval", False, "empty response")
    lines = r.summary_lines()
    text = "\n".join(lines)
    assert "[PASS] claude/reads" in text
    assert "[FAIL] claude/approval" in text
    assert "tool observed server-side: search" in text
    assert cfg["server"]["base_url"] in text


# --- hardened seams -----------------------------------------------------

class _FakePage:
    """Minimal page double for the auth marker state machine."""
    def __init__(self, url, present=()):
        self.url = url
        self._present = set(present)

    def wait_for_selector(self, sel, timeout=0, state=None):
        if sel in self._present:
            return object()
        raise Exception("timeout")


class _FakeProvider:
    def auth_markers(self):
        return (
            {"composer": ["div[contenteditable]"], "menu": ["#menu"]},
            {"login": ["#login-btn"], "login_url": []},
            {"captcha": ["#captcha"]},
        )

    def logged_out_url(self, url):
        return "/login" in (url or "")


def test_auth_verdict_logged_in_needs_two_markers():
    from harness.auth import check_auth_markers
    # Only ONE marker present -> unknown, not logged-in (no false positive
    # on a cached shell showing a single element).
    v = check_auth_markers(_FakePage("https://claude.ai/new",
                                     present=["div[contenteditable]"]),
                           _FakeProvider())
    assert v["verdict"] == "unknown", v
    assert v["missing"] == ["menu"]
    # Both present -> logged-in.
    v = check_auth_markers(_FakePage("https://claude.ai/new",
                                     present=["div[contenteditable]", "#menu"]),
                           _FakeProvider())
    assert v["verdict"] == "logged-in", v
    assert len(v["seen"]) == 2


def test_auth_verdict_logged_out_and_bot():
    from harness.auth import check_auth_markers
    v = check_auth_markers(_FakePage("https://claude.ai/login"), _FakeProvider())
    assert v["verdict"] == "logged-out", v
    v = check_auth_markers(_FakePage("https://claude.ai/new",
                                     present=["#login-btn"]), _FakeProvider())
    assert v["verdict"] == "logged-out", v
    v = check_auth_markers(_FakePage("https://claude.ai/new",
                                     present=["#captcha"]), _FakeProvider())
    assert v["verdict"] == "bot-detected", v


def test_selector_registries_have_fallbacks():
    from harness.providers import selectors_claude as sc, selectors_chatgpt as sg
    for mod in (sc, sg):
        for name in ("LOGGED_IN_MARKERS", "LOGGED_OUT_MARKERS", "BOT_MARKERS"):
            d = getattr(mod, name)
            assert isinstance(d, dict) and d, f"{mod.__name__}.{name}"
            assert len(d["composer"] if "composer" in d else d.get("login_button", ["x"])) >= 1
        # logged-in needs at least two independent keys
        assert len(mod.LOGGED_IN_MARKERS) >= 2


def test_probe_rejects_stale_entry():
    """A tool call logged BEFORE the mark must not count."""
    import time
    with tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False) as f:
        old = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(time.time() - 3600))
        f.write(f'{old}Z {{"method":"tools/call","params":{{"name":"search"}}}}\n')
        path = f.name
    cfg = config_mod.load()
    cfg["server"]["log_file"] = path
    probe = ServerProbe(cfg, skew_allowance=5.0)
    probe.mark()          # mark is NOW; entry is an hour old
    probe._pos = 0        # force re-read of the stale line
    try:
        probe.wait_for_tool("search", timeout=1.0, settle_s=0)
    except ProbeError as e:
        assert e.signal == TOOL_NOT_CALLED
    else:
        raise AssertionError("stale entry must not correlate")
    os.unlink(path)


def test_probe_accepts_fresh_timestamped_entry():
    import time
    with tempfile.NamedTemporaryFile("w+", suffix=".log", delete=False) as f:
        path = f.name
    cfg = config_mod.load()
    cfg["server"]["log_file"] = path
    probe = ServerProbe(cfg, skew_allowance=5.0)
    probe.mark()
    now = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime())
    with open(path, "a") as f:
        f.write(f'{now}Z {{"method":"tools/call","params":{{"name":"post_message"}}}}\n')
    m = probe.wait_for_tool("post_message", timeout=5.0, settle_s=0)
    assert m.correlation == "timestamp", m.correlation
    os.unlink(path)


def test_writes_suite_gated_on_auth_mode():
    from harness.cli import _suites_for_run
    cfg = config_mod.load()
    assert cfg["server"].get("auth_mode") == "no-auth"
    names, skipped = _suites_for_run(cfg)
    assert "writes" in skipped and "writes" not in names
    assert "reads" in names
    cfg["server"]["auth_mode"] = "oauth-stub"
    names, skipped = _suites_for_run(cfg)
    assert "writes" in names and "writes" not in skipped
    # elicitation stays skipped without an elicitation-capable provider.


def test_new_signals_distinct():
    from harness import (KEY_MISSING, BINARY_MISSING, MANUAL_ONLY,
                         CONFIG_ERROR, CODE_TIMEOUT,
                         EXIT_KEY, EXIT_BINARY, EXIT_MANUAL, EXIT_CONFIG,
                         EXIT_CODE_TIMEOUT)
    signals = [KEY_MISSING, BINARY_MISSING, MANUAL_ONLY, CONFIG_ERROR,
               CODE_TIMEOUT]
    assert len(set(signals)) == len(signals)
    codes = [EXIT_KEY, EXIT_BINARY, EXIT_MANUAL, EXIT_CONFIG, EXIT_CODE_TIMEOUT]
    assert len(set(codes)) == len(codes) and codes == [8, 9, 10, 11, 13]
    # And distinct from the original five.
    from harness import EXIT_AUTH, EXIT_SETUP, EXIT_SELECTOR, EXIT_SERVER, EXIT_TOOL
    assert not (set(codes) & {EXIT_AUTH, EXIT_SETUP, EXIT_SELECTOR,
                              EXIT_SERVER, EXIT_TOOL})


def test_all_providers_known():
    from harness.providers import PROVIDERS
    for name in ("grok", "gemini", "copilot-mcp", "copilot-plugin",
                 "claude-code", "cursor", "meta", "claude", "chatgpt"):
        assert name in PROVIDERS, name
        assert get_provider(name, config_mod.load()).name == name


def test_elicitation_only_claude_code():
    """Hard requirement: elicitation tests exist ONLY on Claude Code."""
    from harness.providers import CLI_PROVIDERS, BROWSER_PROVIDERS
    for name, cls in {**CLI_PROVIDERS, **BROWSER_PROVIDERS}.items():
        flag = bool(getattr(cls, "supports_elicitation", False))
        if name == "claude-code":
            assert flag, "claude-code must support it"
        else:
            assert not flag, f"{name} must NOT support elicitation"


def test_elicitation_suite_gated():
    from harness.cli import _suites_for_run
    from harness.providers import get_provider
    cfg = config_mod.load()
    cc = get_provider("claude-code", cfg)
    # Disabled by default -> skipped even on claude-code.
    names, skipped = _suites_for_run(cfg, cc, "elicitation")
    assert names == [] and skipped == ["elicitation"]
    # Enabled but wrong provider -> skipped.
    cfg["elicitation"]["enabled"] = True
    grok = get_provider("grok", cfg)
    names, skipped = _suites_for_run(cfg, grok, "elicitation")
    assert names == [] and skipped == ["elicitation"]
    # Enabled + claude-code -> runs.
    names, skipped = _suites_for_run(cfg, cc, "elicitation")
    assert names == ["elicitation"] and skipped == []


def test_grok_request_shape():
    """Grok request construction: no network, no key needed."""
    from harness.providers import get_provider
    cfg = config_mod.load()
    grok = get_provider("grok", cfg)
    body = grok.build_request_body("hello")
    assert body["model"] == "grok-4.7"
    assert body["input"] == "hello"
    tools = body["tools"]
    assert len(tools) == 1
    t = tools[0]
    assert t["server_url"].endswith("/mcp")
    assert t["server_label"] == "maidan"
    # Never use the unsupported params (research §1.6).
    assert "require_approval" not in t and "connector_id" not in t


def test_grok_refuses_loopback():
    from harness import CONFIG_ERROR
    from harness.providers import get_provider
    from harness.providers.cli_base import ConfigError, EnvError
    cfg = config_mod.load()
    cfg["server"]["base_url"] = "http://127.0.0.1:8080"
    grok = get_provider("grok", cfg)
    try:
        grok._public_url_or_die()
    except ConfigError as e:
        # Honest signal: CONFIG_ERROR, never KEY_MISSING; the message names
        # the URL problem and the remediation (public host or tunnel).
        assert CONFIG_ERROR in str(e)
        assert "KEY_MISSING" not in str(e)
        assert "loopback" in str(e)
        assert "127.0.0.1:8080" in str(e)
    except EnvError:
        raise AssertionError("loopback refusal must not be EnvError/KEY_MISSING")
    else:
        raise AssertionError("loopback URL should fail fast")


def test_cli_env_check():
    import os
    from harness.providers import get_provider
    from harness.providers.cli_base import EnvError
    cfg = config_mod.load()
    grok = get_provider("grok", cfg)
    old = os.environ.pop("XAI_API_KEY", None)
    # Neutralize the vault fallback: with neither env nor vault, the key
    # is genuinely missing.
    grok._vault_surrogate = lambda: None
    try:
        try:
            grok.check_env()
        except EnvError as e:
            assert e.var == "XAI_API_KEY"
        else:
            raise AssertionError("missing key should raise EnvError")
    finally:
        if old is not None:
            os.environ["XAI_API_KEY"] = old


def test_grok_vault_fallback():
    """Key resolution order: explicit env key wins, vault is the fallback."""
    import os
    from harness.providers import get_provider
    cfg = config_mod.load()
    grok = get_provider("grok", cfg)
    old = os.environ.get("XAI_API_KEY")
    try:
        os.environ["XAI_API_KEY"] = "env-key-wins"
        assert grok._auth_header() == "Bearer env-key-wins"
        del os.environ["XAI_API_KEY"]
        grok._vault_surrogate = lambda: "hsurr:test-surrogate"
        assert grok._auth_header() == "Bearer hsurr:test-surrogate"
    finally:
        if old is not None:
            os.environ["XAI_API_KEY"] = old
        else:
            os.environ.pop("XAI_API_KEY", None)


def test_cli_env_any_of():
    """required_env_any: at least one of the token names must be set."""
    import os
    from harness.providers import get_provider
    from harness.providers.cli_base import EnvError
    cfg = config_mod.load()
    copilot = get_provider("copilot-mcp", cfg)
    vars_ = ("COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")
    saved = {v: os.environ.pop(v, None) for v in vars_}
    try:
        try:
            copilot.check_env()
        except EnvError:
            pass
        else:
            raise AssertionError("no token set should raise EnvError")
        os.environ["GH_TOKEN"] = "dummy"
        copilot.check_env()  # one of three is enough
    finally:
        for v, old in saved.items():
            if old is not None:
                os.environ[v] = old
            else:
                os.environ.pop(v, None)


def test_manual_only_providers():
    from harness.providers import get_provider
    cfg = config_mod.load()
    for name in ("cursor", "meta"):
        assert get_provider(name, cfg).manual_only, name
    assert get_provider("meta", cfg).checklist_only, "meta"
    assert not get_provider("cursor", cfg).checklist_only, "cursor"
    for name in ("grok", "gemini", "claude-code", "copilot-mcp",
                 "copilot-plugin"):
        assert not get_provider(name, cfg).manual_only, name


def test_suite_only_param():
    """Regression: --suite <name> must select exactly that suite (D1).

    _suites_for_run(cfg, prov, only) takes the provider positionally; a
    call that binds the suite name to `prov` silently runs everything.
    """
    from harness.cli import _suites_for_run
    from harness.providers import get_provider
    cfg = config_mod.load()
    prov = get_provider("claude", cfg)
    names, skipped = _suites_for_run(cfg, prov, "reads")
    assert names == ["reads"], names
    assert skipped == [], skipped
    # And "all" still returns every runnable suite (no silent narrowing).
    names, _ = _suites_for_run(cfg, prov, "all")
    assert set(names) == {"reads", "multi_tool", "approval"}, names


# --- magic-link tier ----------------------------------------------------

# Realistic Claude sign-in email shape (verified 2026-10-07 against a real
# email: sender zyla@beatgig.com via the Zyla relay, subject "Your secure link
# to Claude.ai is here | <timestamp>", HTML body with a "Sign in" button
# linking to claude.ai).
_CLAUDE_FIXTURE_HTML = """\
<html><body>
<p>Click the link below to log in to Claude.ai. It expires soon.</p>
<a href="https://claude.ai/login/verify?token=SECRET-LINK-ABC123">Sign in to Claude.ai</a>
<p>Or paste this URL: https://claude.ai/login/verify?token=SECRET-LINK-ABC123</p>
<a href="https://support.anthropic.com">Help center</a>
</body></html>"""

# Realistic ChatGPT sign-in shape (UNVERIFIED: sender/subject guessed from
# public email reports; confirm against the first real email before
# enabling). Mechanism is link-based per David's correction, 2026-10-07.
_CHATGPT_FIXTURE_HTML = """\
<html><body>
<p>Click the button below to sign in to ChatGPT.</p>
<a href="https://auth.openai.com/authorize/continue?ticket=CHATGPT-LINK-XYZ789">Sign in</a>
<p>This link expires in 30 minutes.</p>
</body></html>"""


def _ml_meta(mid, subject, body, age_s=5):
    import time
    return {"id": mid, "from": "x", "subject": subject,
            "internal_ms": int((time.time() - age_s) * 1000), "body": body}


def test_magiclink_claude_link_extract():
    from harness.magiclink import poll_for_signin
    fetch = lambda q: [_ml_meta(
        "m1", "Your secure link to Claude.ai is here | 2026-10-07 11:00:00",
        _CLAUDE_FIXTURE_HTML)]
    import time
    s = poll_for_signin("claude", "zyla@beatgig.com",
                        time.time() - 60, timeout=5, poll_interval=0.1,
                        _fetch=fetch)
    assert s.url == "https://claude.ai/login/verify?token=SECRET-LINK-ABC123"
    assert s.message_id == "m1"


def test_magiclink_claude_newest_wins():
    from harness.magiclink import poll_for_signin
    import time
    now = time.time()
    fetch = lambda q: [
        _ml_meta("old", "Your secure link to Claude.ai is here | 2026-10-07 10:00:00",
                 _CLAUDE_FIXTURE_HTML.replace("ABC123", "OLD"), age_s=600),
        _ml_meta("new", "Your secure link to Claude.ai is here | 2026-10-07 11:00:00",
                 _CLAUDE_FIXTURE_HTML, age_s=5),
    ]
    s = poll_for_signin("claude", "zyla@beatgig.com",
                        now - 60, timeout=5, poll_interval=0.1, _fetch=fetch)
    assert s.message_id == "new"


def test_magiclink_chatgpt_link_extract():
    # ChatGPT's shape is UNVERIFIED: extraction requires the explicit
    # override plus the operator-confirmed domain allowlist.
    from harness.magiclink import poll_for_signin
    import time
    fetch = lambda q: [_ml_meta(
        "m2", "Sign in to ChatGPT", _CHATGPT_FIXTURE_HTML)]
    s = poll_for_signin("chatgpt", "zyla@beatgig.com",
                        time.time() - 60, timeout=5, poll_interval=0.1,
                        allow_unverified=True,
                        link_domains=("auth.openai.com",),
                        _fetch=fetch)
    assert s.url == ("https://auth.openai.com/authorize/continue"
                     "?ticket=CHATGPT-LINK-XYZ789")
    assert s.message_id == "m2"


def test_magiclink_unverified_refused():
    """Unverified providers refuse to run without the explicit override."""
    from harness.magiclink import (poll_for_signin, MagicLinkError,
                                   MagicLinkConfigError)
    import time
    try:
        poll_for_signin("chatgpt", "zyla@beatgig.com", time.time(),
                        timeout=1, poll_interval=0.1,
                        _fetch=lambda q: [])
    except MagicLinkConfigError:
        pass
    else:
        raise AssertionError("unverified provider should be refused")
    assert issubclass(MagicLinkConfigError, MagicLinkError)


def test_magiclink_empty_allowlist_refused():
    """Even with the override, an empty domain allowlist is fail-closed."""
    from harness.magiclink import poll_for_signin, MagicLinkConfigError
    import time
    try:
        poll_for_signin("chatgpt", "zyla@beatgig.com", time.time(),
                        timeout=1, poll_interval=0.1,
                        allow_unverified=True, _fetch=lambda q: [])
    except MagicLinkConfigError:
        return
    raise AssertionError("empty allowlist should be refused")


def test_magiclink_evil_domain_rejected():
    """claude.ai.evil.com must NOT satisfy the claude.ai allowlist."""
    from harness.magiclink import _extract_link, _host_ok
    evil = ('<a href="https://claude.ai.evil.com/verify?token=x">'
            "Sign in</a>")
    assert not _host_ok("https://claude.ai.evil.com/verify?token=x",
                        ("claude.ai",))
    assert _extract_link(evil, ("claude.ai",), r"sign\s*in") is None
    # ... and the any-https fallback is gone: unknown domains never match.
    assert _extract_link(
        '<a href="https://tracker.example.com/r">Sign in</a>',
        ("claude.ai",), r"sign\s*in") is None
    # Real subdomains still work.
    assert _host_ok("https://auth.claude.ai/x", ("claude.ai",))


def test_magiclink_bare_url_trailing_punctuation():
    """Pass-3 bare URLs strip trailing punctuation (m3)."""
    from harness.magiclink import _extract_link
    html = "paste this: https://claude.ai/login/verify?token=abc123."
    assert _extract_link(html, ("claude.ai",),
                         r"verify") == "https://claude.ai/login/verify?token=abc123"


def test_egress_proxy_target_parsing():
    import os
    from harness.browser import _egress_proxy_target
    orig = dict(os.environ)
    try:
        for var in ("https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"):
            os.environ.pop(var, None)
        assert _egress_proxy_target() is None
        os.environ["https_proxy"] = "hatch-egress-proxy:3128"
        assert _egress_proxy_target() == ("hatch-egress-proxy", 3128)
        os.environ["https_proxy"] = "http://user:pass@proxy.example.com:8080/"
        assert _egress_proxy_target() == ("proxy.example.com", 8080)
        os.environ["https_proxy"] = "not-a-proxy"
        assert _egress_proxy_target() is None
    finally:
        os.environ.clear()
        os.environ.update(orig)


def test_sandbox_mitm_uses_exact_matching():
    """The TLS-downgrade gate must use exact hostname matching, never
    substring: a proxy at evil-hatch-example.com must NOT trigger
    ignore_https_errors."""
    import os
    from harness import browser as browser_mod
    orig = os.environ.get("SSL_CERT_FILE")
    os.environ.pop("SSL_CERT_FILE", None)
    try:
        # Substring trap: contains "hatch" but is not the sandbox proxy.
        assert browser_mod._is_sandbox_mitm(("evil-hatch-example.com", 8080)) is False
        # Exact host match triggers.
        assert browser_mod._is_sandbox_mitm(("hatch-egress-proxy", 8080)) is True
        # Subdomain of the exact host triggers.
        assert browser_mod._is_sandbox_mitm(("x.hatch-egress-proxy", 8080)) is True
    finally:
        if orig is not None:
            os.environ["SSL_CERT_FILE"] = orig


def test_proxy_relay_forwards():
    """_ProxyRelay forwards bytes to the target (sandbox workaround)."""
    import socket, threading, time
    from harness.browser import _ProxyRelay
    received = []

    def echo_server(srv):
        c, _ = srv.accept()
        data = c.recv(4096)
        received.append(data)
        c.sendall(b"echo:" + data)
        c.close()
        srv.close()

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    port = srv.getsockname()[1]
    threading.Thread(target=echo_server, args=(srv,), daemon=True).start()

    relay = _ProxyRelay(("127.0.0.1", port))
    try:
        s = socket.create_connection(("127.0.0.1", relay.port), timeout=10)
        # Realistic HTTP CONNECT, sent in two fragments to prove the
        # relay reassembles before the single upstream write.
        s.sendall(b"CONNECT example.com:443 HTTP/1.1\r\nHost: exa")
        s.sendall(b"mple.com:443\r\n\r\n")
        resp = s.recv(4096)
        s.close()
        assert resp == b"echo:CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n", resp
        # The two fragments arrived as ONE upstream write (reassembled).
        assert received == [b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n"]
    finally:
        relay.close()


def test_gws_nonjson_surfaces_stdout():
    """Exit-0 non-JSON stdout keeps the CLI's own error text (m2)."""
    import subprocess
    import harness.magiclink as ml_mod
    from harness.magiclink import MagicLinkError

    class FakeCompleted:
        returncode = 0
        stdout = "Error: the account may not be linked\n"
        stderr = ""

    orig_run = subprocess.run
    subprocess.run = lambda *a, **k: FakeCompleted()
    try:
        ml_mod._gws_json(["gmail"], account="bogus")
    except MagicLinkError as e:
        assert "may not be linked" in str(e), str(e)
        return
    finally:
        subprocess.run = orig_run
    raise AssertionError("non-JSON stdout should raise MagicLinkError")


def test_nightly_auth_heal_sys_exit_isolated():
    """A heal that exits AUTH_EXPIRED fails just that provider (M2)."""
    import harness.cli as cli_mod
    from harness import EXIT_AUTH

    def fake_check(provider, cfg):
        raise SystemExit(EXIT_AUTH)

    def fake_heal(provider, cfg):
        raise SystemExit(EXIT_AUTH)

    class FakeReport:
        def __init__(self):
            self.failures = []
        def event(self, kind, **kw):
            pass
        def failure(self, signal, detail=""):
            self.failures.append(signal)

    orig_check, orig_heal = cli_mod.check_auth_or_die, cli_mod.magic_link_login
    cli_mod.check_auth_or_die, cli_mod.magic_link_login = fake_check, fake_heal
    try:
        cfg = config_mod.load()
        cfg["providers"]["claude"]["magic_link"]["enabled"] = True
        rep = FakeReport()
        # Must NOT propagate SystemExit (that would abort the whole nightly).
        assert cli_mod._nightly_auth("claude", cfg, rep) is None
        assert "AUTH_EXPIRED" in rep.failures
    finally:
        cli_mod.check_auth_or_die, cli_mod.magic_link_login = orig_check, orig_heal


def test_magiclink_timeout():
    from harness.magiclink import poll_for_signin, CodeTimeout
    import time
    try:
        poll_for_signin("claude", "nobody@example.com", time.time(),
                        timeout=2, poll_interval=0.2,
                        _fetch=lambda q: [])
    except CodeTimeout:
        return
    raise AssertionError("empty inbox should raise CodeTimeout")


def test_magiclink_stale_mail_ignored():
    from harness.magiclink import poll_for_signin, CodeTimeout
    import time
    # A matching-subject mail from yesterday must NOT be used.
    fetch = lambda q: [_ml_meta(
        "stale", "Your secure link to Claude.ai is here | 2026-10-06 11:00:00",
        _CLAUDE_FIXTURE_HTML, age_s=90000)]
    try:
        poll_for_signin("claude", "zyla@beatgig.com",
                        time.time(), timeout=2, poll_interval=0.2,
                        _fetch=fetch)
    except CodeTimeout:
        return
    raise AssertionError("stale mail should be ignored -> CodeTimeout")


def test_magiclink_no_secret_in_logs():
    """The sign-in link must never appear on stdout/stderr, even in
    debug paths (it is a bearer credential)."""
    import io
    import time
    from contextlib import redirect_stdout, redirect_stderr
    from harness.magiclink import poll_for_signin
    fetch = lambda q: [_ml_meta(
        "m1", "Your secure link to Claude.ai is here | 2026-10-07 11:00:00",
        _CLAUDE_FIXTURE_HTML)]
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        s = poll_for_signin("claude", "zyla@beatgig.com",
                            time.time() - 60, timeout=5, poll_interval=0.1,
                            _fetch=fetch)
    captured = out.getvalue() + err.getvalue()
    assert "SECRET-LINK-ABC123" not in captured, "link leaked into logs"
    assert s.url == "https://claude.ai/login/verify?token=SECRET-LINK-ABC123"


def test_magiclink_account_plumbed():
    """The configured Gmail account_id reaches `hatch_gws_cli --account`."""
    import json
    import time
    import harness.magiclink as ml_mod
    seen = {}

    class FakeResult:
        returncode = 0
        stdout = json.dumps({"messages": []})
        stderr = ""

    def fake_run(cmd, **kw):
        seen["cmd"] = cmd
        return FakeResult()

    orig = ml_mod.subprocess.run
    ml_mod.subprocess.run = fake_run
    try:
        ml_mod._gws_json(["gmail", "users", "labels", "list"],
                         account="acct-123")
    finally:
        ml_mod.subprocess.run = orig
    assert seen["cmd"][-2:] == ["--account", "acct-123"], seen["cmd"]

    # And None means no --account flag at all.
    def fake_run2(cmd, **kw):
        seen["cmd2"] = cmd
        return FakeResult()

    ml_mod.subprocess.run = fake_run2
    try:
        ml_mod._gws_json(["gmail", "users", "labels", "list"], account=None)
    finally:
        ml_mod.subprocess.run = orig
    assert "--account" not in seen["cmd2"], seen["cmd2"]


def test_magiclink_config_defaults():
    """Config defaults: zyla@beatgig.com, disabled, Beatgig account pinned.

    The account id is the linked david@beatgig.com Gmail account, verified
    2026-10-07 to see zyla@beatgig.com mail (zyla@ is an alias delivering
    there)."""
    cfg = config_mod.load()
    for provider in ("claude", "chatgpt"):
        ml = cfg["providers"][provider]["magic_link"]
        assert ml["enabled"] is False, provider
        assert ml["email"] == "zyla@beatgig.com", provider
        assert ml.get("account") == "9b411cec9abc4d12bffaaa5c001df53f", provider


def test_magiclink_unknown_provider():
    from harness.magiclink import poll_for_signin, MagicLinkError
    import time
    try:
        poll_for_signin("grok", "x@example.com", time.time(),
                        timeout=1, poll_interval=0.1, _fetch=lambda q: [])
    except MagicLinkError:
        return
    raise AssertionError("unknown provider should raise MagicLinkError")


def test_magiclink_one_shot_override():
    """--via-email (force=True) bypasses the enabled gate; default doesn't."""
    from harness.auth import _magic_link_config
    from harness.magiclink import MagicLinkConfigError
    cfg = config_mod.load()  # claude magic_link.enabled is False
    try:
        _magic_link_config("claude", cfg)
    except MagicLinkConfigError:
        pass
    else:
        raise AssertionError("default path should require enabled")
    ml = _magic_link_config("claude", cfg, require_enabled=False)
    assert ml["email"] == "zyla@beatgig.com"


def test_magiclink_config_error_is_config_error():
    """Config-shaped failures are MagicLinkConfigError (CONFIG_ERROR/11),
    not plain MagicLinkError (TEST_FAILED/1)."""
    from harness.auth import _magic_link_config
    from harness.magiclink import MagicLinkConfigError, MagicLinkError
    cfg = config_mod.load()
    # Disabled by default.
    try:
        _magic_link_config("claude", cfg)
    except MagicLinkConfigError:
        pass
    else:
        raise AssertionError("disabled tier should raise MagicLinkConfigError")
    # It is still a MagicLinkError (catch sites stay correct).
    assert issubclass(MagicLinkConfigError, MagicLinkError)


def test_nightly_auth_single_heal():
    """_nightly_auth: exactly one heal attempt, one re-check, then stop."""
    import harness.cli as cli_mod
    from harness import EXIT_AUTH
    calls = {"check": 0, "heal": 0}

    def fake_check(provider, cfg):
        calls["check"] += 1
        if calls["check"] == 1:
            raise SystemExit(EXIT_AUTH)
        return {"verdict": "logged-in"}

    def fake_heal(provider, cfg):
        calls["heal"] += 1
        return {"verdict": "logged-in"}

    class FakeReport:
        def __init__(self):
            self.events = []
            self.failures = []
        def event(self, kind, **kw):
            self.events.append(kind)
        def failure(self, signal, detail=""):
            self.failures.append(signal)

    orig_check, orig_heal = cli_mod.check_auth_or_die, cli_mod.magic_link_login
    cli_mod.check_auth_or_die, cli_mod.magic_link_login = fake_check, fake_heal
    try:
        cfg = config_mod.load()
        cfg["providers"]["claude"]["magic_link"]["enabled"] = True
        rep = FakeReport()
        verdict = cli_mod._nightly_auth("claude", cfg, rep)
        assert verdict == {"verdict": "logged-in"}
        assert calls == {"check": 2, "heal": 1}, calls
        assert "auth_heal_attempt" in rep.events
        assert "auth_heal_done" in rep.events
        assert rep.failures == []
    finally:
        cli_mod.check_auth_or_die, cli_mod.magic_link_login = orig_check, orig_heal


def test_nightly_auth_heal_still_expired():
    """Heal attempted once; still expired -> failure recorded, no loop."""
    import harness.cli as cli_mod
    from harness import EXIT_AUTH
    calls = {"check": 0, "heal": 0}

    def fake_check(provider, cfg):
        calls["check"] += 1
        raise SystemExit(EXIT_AUTH)

    def fake_heal(provider, cfg):
        calls["heal"] += 1
        return {"verdict": "logged-in"}

    class FakeReport:
        def __init__(self):
            self.failures = []
        def event(self, kind, **kw):
            pass
        def failure(self, signal, detail=""):
            self.failures.append(signal)

    orig_check, orig_heal = cli_mod.check_auth_or_die, cli_mod.magic_link_login
    cli_mod.check_auth_or_die, cli_mod.magic_link_login = fake_check, fake_heal
    try:
        cfg = config_mod.load()
        cfg["providers"]["claude"]["magic_link"]["enabled"] = True
        rep = FakeReport()
        assert cli_mod._nightly_auth("claude", cfg, rep) is None
        assert calls == {"check": 2, "heal": 1}, calls  # no retry loop
        assert "AUTH_EXPIRED" in rep.failures
    finally:
        cli_mod.check_auth_or_die, cli_mod.magic_link_login = orig_check, orig_heal


def test_nightly_auth_no_heal_when_disabled():
    """magic_link disabled -> straight to AUTH_EXPIRED failure, no heal."""
    import harness.cli as cli_mod
    from harness import EXIT_AUTH
    calls = {"check": 0, "heal": 0}

    def fake_check(provider, cfg):
        calls["check"] += 1
        raise SystemExit(EXIT_AUTH)

    def fake_heal(provider, cfg):
        calls["heal"] += 1

    class FakeReport:
        def __init__(self):
            self.failures = []
        def event(self, kind, **kw):
            pass
        def failure(self, signal, detail=""):
            self.failures.append(signal)

    orig_check, orig_heal = cli_mod.check_auth_or_die, cli_mod.magic_link_login
    cli_mod.check_auth_or_die, cli_mod.magic_link_login = fake_check, fake_heal
    try:
        cfg = config_mod.load()
        cfg["providers"]["claude"]["magic_link"]["enabled"] = False
        rep = FakeReport()
        assert cli_mod._nightly_auth("claude", cfg, rep) is None
        assert calls == {"check": 1, "heal": 0}, calls
        assert rep.failures == ["AUTH_EXPIRED"]
    finally:
        cli_mod.check_auth_or_die, cli_mod.magic_link_login = orig_check, orig_heal


if __name__ == "__main__":
    fns = [v for k, v in sorted(globals().items())
           if k.startswith("test_") and callable(v)]
    failed = 0
    for fn in fns:
        try:
            fn()
            print(f"PASS {fn.__name__}")
        except Exception as e:
            failed += 1
            print(f"FAIL {fn.__name__}: {e}")
    sys.exit(1 if failed else 0)
