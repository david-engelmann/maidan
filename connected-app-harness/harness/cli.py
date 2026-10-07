"""harness CLI.

Commands:
  auth-login <provider>   One-time manual login (headed). David runs once.
  check-auth <provider>   Fail fast with AUTH_EXPIRED unless session alive.
  setup <provider>        Install connector/plugin if missing (idempotent).
  setup-verify <provider> Check install only; SETUP_NEEDED if missing.
  test <provider> [--suite NAME|all]
                         New chat -> prompt -> server-side assertions.
  nightly                 check-auth + setup-verify + all tests, all
                         providers, timestamped report. Cron-friendly:
                         zero human in the loop when auth is fresh.
  config-sha <sha>        Record the server commit SHA in config.yaml.

Exit codes: 0 ok · 1 test failure · 3 AUTH_EXPIRED · 4 SETUP_NEEDED ·
5 SELECTOR_STALE · 6 SERVER_UNREACHABLE · 7 TOOL_NOT_CALLED ·
8 KEY_MISSING · 9 BINARY_MISSING · 10 MANUAL_ONLY · 11 CONFIG_ERROR.
Every failure also prints its greppable signal token on stderr.
"""
from __future__ import annotations

import argparse
import os
import sys

from . import (
    AUTH_EXPIRED, SETUP_NEEDED, TEST_FAILED, KEY_MISSING, BINARY_MISSING,
    MANUAL_ONLY, CODE_TIMEOUT, CONFIG_ERROR,
    EXIT_AUTH, EXIT_SETUP, EXIT_KEY, EXIT_BINARY, EXIT_MANUAL,
    EXIT_CODE_TIMEOUT, EXIT_CONFIG,
)
from . import config as config_mod
from .auth import auth_login, check_auth_or_die, magic_link_login
from .browser import persistent_context
from .magiclink import MagicLinkError, MagicLinkConfigError, CodeTimeout
from .probe import ServerProbe, ProbeError, die as probe_die
from .providers import (
    get_provider, is_cli_provider, BROWSER_PROVIDERS, CLI_PROVIDERS,
    NIGHTLY_ORDER,
)
from .providers.cli_base import EnvError, ConfigError, BinaryError, die_env, die_config, die_binary
from .reporting import Report


def _provider_or_die(name: str, cfg: dict):
    return get_provider(name, cfg)


def _manual_only_exit(prov, name: str):
    """Refuse with MANUAL_ONLY + the provider's checklist pointer."""
    print(MANUAL_ONLY, file=sys.stderr)
    checklist = getattr(prov, "checklist_path", f"checklists/{name}.md")
    print(f"{name}: cannot run unattended; see {checklist}",
          file=sys.stderr)
    sys.exit(EXIT_MANUAL)


def _cli_setup_or_die(prov, name: str):
    """Gate for setup/setup-verify: manual_only providers with a real config
    writer (cursor) may proceed; checklist-only providers (meta) refuse."""
    if getattr(prov, "checklist_only", False):
        _manual_only_exit(prov, name)
    return prov


def _cli_run_or_die(prov, name: str):
    """Gate for auth-login/test/nightly: manual_only providers refuse."""
    if getattr(prov, "manual_only", False):
        _manual_only_exit(prov, name)
    return prov


def _cli_env_or_die(prov) -> None:
    try:
        prov.check_env()
    except EnvError as e:
        die_env(e)


def _cli_binary_or_die(prov) -> None:
    try:
        prov.ensure_binary()
    except BinaryError as e:
        die_binary(e)


def cmd_auth_login(args, cfg):
    prov = _provider_or_die(args.provider, cfg)
    if is_cli_provider(args.provider):
        _cli_run_or_die(prov, args.provider)
        print(f"{args.provider}: no browser login. Prerequisites:",
              file=sys.stderr)
        if prov.required_env:
            print(f"  export {' '.join(prov.required_env)}=<value>",
                  file=sys.stderr)
        if prov.binary:
            print(f"  binary '{prov.binary}' (auto-installed on first use)",
                  file=sys.stderr)
        print("See README.md for install instructions.", file=sys.stderr)
        sys.exit(1)
    ml = ((cfg.get("providers") or {}).get(args.provider) or {}).get("magic_link") or {}
    if args.via_email or ml.get("enabled", False):
        # Magic-link flow: headless, zero human. Becomes the default once
        # providers.<name>.magic_link.enabled=true in config.
        try:
            verdict = magic_link_login(args.provider, cfg,
                                       bypass_enabled_gate=args.via_email)
        except CodeTimeout as e:
            print(CODE_TIMEOUT, file=sys.stderr)
            print(f"{args.provider}: {e}", file=sys.stderr)
            sys.exit(EXIT_CODE_TIMEOUT)
        except MagicLinkConfigError as e:
            print(CONFIG_ERROR, file=sys.stderr)
            print(f"{args.provider}: {e}", file=sys.stderr)
            sys.exit(EXIT_CONFIG)
        except MagicLinkError as e:
            print(f"{TEST_FAILED}: magic-link login: {e}", file=sys.stderr)
            sys.exit(1)
        print(f"OK: {args.provider} magic-link session saved "
              f"(markers: {verdict['seen']})")
        return
    auth_login(args.provider, cfg)


def cmd_check_auth(args, cfg):
    prov = _provider_or_die(args.provider, cfg)
    if is_cli_provider(args.provider):
        if getattr(prov, "manual_only", False):
            # No credential to probe on manual providers. The verifiable
            # part is the config wiring (cursor); checklist-only providers
            # (meta) have nothing to check.
            if getattr(prov, "checklist_only", False) or not prov.is_setup():
                _manual_only_exit(prov, args.provider)
            print(f"OK: {args.provider} config present; IDE auth is manual "
                  f"(see {prov.checklist_path})")
            return
        _cli_env_or_die(prov)
        _cli_binary_or_die(prov)
        print(f"OK: {args.provider} env/binary ready")
        return
    check_auth_or_die(args.provider, cfg)
    print(f"OK: {args.provider} session alive")


def cmd_setup(args, cfg):
    prov = _provider_or_die(args.provider, cfg)
    if is_cli_provider(args.provider):
        _cli_setup_or_die(prov, args.provider)
        _cli_env_or_die(prov)
        _cli_binary_or_die(prov)
        if prov.is_setup():
            print(f"OK: {args.provider} already set up")
            return
        print(f"installing {args.provider}...")
        try:
            prov.install()
        except EnvError as e:
            die_env(e)
        except ConfigError as e:
            die_config(e)
        if prov.is_setup():
            print(f"OK: {args.provider} setup complete")
        else:
            print(SETUP_NEEDED, file=sys.stderr)
            print(f"{args.provider}: install did not take; retry setup",
                  file=sys.stderr)
            sys.exit(EXIT_SETUP)
        return
    check_auth_or_die(args.provider, cfg)
    with persistent_context(args.provider, headless=True) as ctx:
        page = ctx.new_page()
        if prov.is_setup(page):
            print(f"OK: {args.provider} already set up")
            return
        print(f"installing {args.provider} connector/plugin...")
        prov.install(page)
        if prov.is_setup(page):
            print(f"OK: {args.provider} setup complete")
        else:
            print(SETUP_NEEDED, file=sys.stderr)
            print(f"{args.provider}: install did not take; retry setup", file=sys.stderr)
            sys.exit(EXIT_SETUP)


def cmd_setup_verify(args, cfg):
    prov = _provider_or_die(args.provider, cfg)
    if is_cli_provider(args.provider):
        _cli_setup_or_die(prov, args.provider)
        _cli_env_or_die(prov)
        _cli_binary_or_die(prov)
        if prov.is_setup():
            print(f"OK: {args.provider} setup verified")
            return
        print(SETUP_NEEDED, file=sys.stderr)
        print(f"{args.provider}: not installed; run `harness setup {args.provider}`",
              file=sys.stderr)
        sys.exit(EXIT_SETUP)
    check_auth_or_die(args.provider, cfg)
    with persistent_context(args.provider, headless=True) as ctx:
        page = ctx.new_page()
        if prov.is_setup(page):
            print(f"OK: {args.provider} setup verified")
            return
    print(SETUP_NEEDED, file=sys.stderr)
    print(f"{args.provider}: connector/plugin missing; run `harness setup {args.provider}`",
          file=sys.stderr)
    sys.exit(EXIT_SETUP)


def _run_preflight(args, cfg, prov, probe, report) -> None:
    """MCP preflight + client telemetry, recorded in the report.

    Fails the run (no traceback) when the server is unreachable or
    doesn't expose tools: suites must never run against an unknown
    server surface.
    """
    try:
        probe.check_reachable()
        pre = probe.preflight()
    except ProbeError as e:
        probe_die(e)
    report.event("mcp_preflight", provider=args.provider,
                 tools=pre["tools"], tool_count=pre["count"])
    report.event("client_info", provider=args.provider,
                 surface=prov.surface,
                 auth_mode=cfg["server"].get("auth_mode", "no-auth"))


def _suites_for_run(cfg, prov=None, only: str = "all") -> list[str]:
    names = [s for s in cfg["suites"]]
    if only != "all":
        names = [only]
    # Write-flow auth path is separate from the no-auth read path: the
    # writes suite runs ONLY against an oauth-stub server, otherwise it
    # is skipped (never failed, never conflated).
    # Elicitation is Claude Code only: the suite is refused everywhere else.
    auth_mode = cfg["server"].get("auth_mode", "no-auth")
    elicitation_ok = bool(getattr(prov, "supports_elicitation", False))
    elicitation_enabled = bool((cfg.get("elicitation") or {}).get("enabled", False))
    out = []
    skipped = []
    for n in names:
        spec = cfg["suites"][n]
        if spec.get("requires_auth") and auth_mode != "oauth-stub":
            skipped.append(n)
        elif spec.get("requires_elicitation") and not (elicitation_ok and elicitation_enabled):
            skipped.append(n)
        else:
            out.append(n)
    return out, skipped


def _finish_suite(prov, probe: ServerProbe, report: Report, suite: str,
                  response: str) -> bool:
    """Shared core of _run_suite/_run_cli_suite: server-side tool assertions
    plus the secondary non-empty-response check, then suite_result.

    Never sys.exit — the report is the artifact, even on failure. The only
    contract: the caller obtained `response` (browser round-trip or
    run_prompt) after probe.mark().
    """
    spec = prov.cfg["suites"][suite]
    expect = spec.get("expect_tools") or [spec.get("expect_tool")]
    expect = [t for t in expect if t]

    ok = True
    for tool in expect:
        try:
            # Sequential wait_for_tool calls enforce ORDER: each call
            # advances the probe's log position past its match, so a later
            # tool must appear after the earlier one. Out-of-order calls
            # surface as TOOL_NOT_CALLED for the misplaced tool.
            match = probe.wait_for_tool(tool, timeout=120.0)
            report.tool_called(prov.name, tool, suite,
                               correlation=match.correlation)
        except ProbeError as e:
            # Record and return: the caller writes the report. Never
            # sys.exit here — the report is the artifact, even on failure.
            report.failure(e.signal, e.detail)
            print(e.detail, file=sys.stderr)
            ok = False
    if not ok:
        report.suite_result(prov.name, suite, False, "tool not called")
        return False
    # Secondary assertion: the assistant said something non-empty.
    if not response.strip():
        report.suite_result(prov.name, suite, False, "empty response")
        print(f"{TEST_FAILED}: {prov.name}/{suite}: empty response",
              file=sys.stderr)
        return False
    return True


def _run_suite(prov, page, probe: ServerProbe, report: Report, suite: str) -> bool:
    spec = prov.cfg["suites"][suite]
    prompt = spec["prompt"]

    probe.mark()
    prov.new_chat(page)
    prov.send_prompt(page, prompt)
    response = prov.wait_for_response(page)

    if not _finish_suite(prov, probe, report, suite, response):
        return False
    # Provider-specific advanced checks (best-effort, never fatal).
    widget = getattr(prov, "widget_present", None)
    if callable(widget):
        try:
            if widget(page):
                report.event("widget_rendered", provider=prov.name, suite=suite)
        except Exception:
            pass
    report.suite_result(prov.name, suite, True,
                        f"response {len(response)} chars")
    return True


def _run_cli_suite(prov, probe: ServerProbe, report: Report, suite: str) -> bool:
    """CLI equivalent of _run_suite: run_prompt -> server-side assertions."""
    spec = prov.cfg["suites"][suite]
    prompt = spec["prompt"]

    probe.mark()
    try:
        response = prov.run_prompt(prompt)
    except Exception as e:
        report.failure(TEST_FAILED, f"{prov.name}/{suite}: run_prompt: {e}")
        print(f"{TEST_FAILED}: {prov.name}/{suite}: run_prompt: {e}",
              file=sys.stderr)
        return False

    if not _finish_suite(prov, probe, report, suite, response):
        return False
    report.suite_result(prov.name, suite, True,
                        f"response {len(response)} chars")
    return True


def cmd_test(args, cfg):
    prov = _provider_or_die(args.provider, cfg)
    if is_cli_provider(args.provider):
        _cli_run_or_die(prov, args.provider)
        _cli_env_or_die(prov)
        _cli_binary_or_die(prov)
        return _cmd_test_cli(args, cfg, prov)
    verdict = check_auth_or_die(args.provider, cfg)
    probe = ServerProbe(cfg, skew_allowance=float(cfg["server"].get("skew_allowance", 5.0)))
    report = Report(cfg, f"test-{args.provider}")
    _run_preflight(args, cfg, prov, probe, report)
    report.command(["test", args.provider, "--suite", args.suite])
    report.event("auth_verdict", provider=args.provider, **verdict)

    # test assumes setup; never install mid-run. Cheap probe first:
    # no chat round-trip burned on a missing connector.
    with persistent_context(args.provider, headless=True) as ctx:
        page = ctx.new_page()
        setup = prov.setup_probe(page)
        report.event("setup_probe", provider=args.provider, **setup)
        if not setup["ok"]:
            report.failure(SETUP_NEEDED,
                           f"{args.provider}: {setup['detail']}")
            print(SETUP_NEEDED, file=sys.stderr)
            print(f"{args.provider}: {setup['detail']}; "
                  f"run `harness setup {args.provider}`", file=sys.stderr)
            sys.exit(EXIT_SETUP)
        names, skipped = _suites_for_run(cfg, prov, args.suite)
        for s in skipped:
            report.event("suite_skipped", provider=args.provider, suite=s,
                         reason="requires oauth-stub server")
        failed = []
        for suite in names:
            try:
                if not _run_suite(prov, page, probe, report, suite):
                    failed.append(suite)
            except ProbeError as e:
                # wait_for_tool already reported; keep going to next suite.
                failed.append(suite)
            except SystemExit:
                raise
            except Exception as e:  # selector rot etc. already signaled
                report.failure(TEST_FAILED, f"{args.provider}/{suite}: {e}")
                failed.append(suite)
    path = report.write()
    for line in report.summary_lines():
        print(line)
    print(f"wrote {path}")
    sys.exit(1 if failed else 0)


def _cmd_test_cli(args, cfg, prov) -> None:
    """test for CLI providers: env/binary -> probe -> suites -> report."""
    probe = ServerProbe(cfg, skew_allowance=float(cfg["server"].get("skew_allowance", 5.0)))
    report = Report(cfg, f"test-{args.provider}")
    _run_preflight(args, cfg, prov, probe, report)
    report.command(["test", args.provider, "--suite", args.suite])

    if not prov.is_setup():
        report.failure(SETUP_NEEDED, f"{args.provider}: not installed")
        print(SETUP_NEEDED, file=sys.stderr)
        print(f"{args.provider}: not installed; run `harness setup {args.provider}`",
              file=sys.stderr)
        sys.exit(EXIT_SETUP)
    names, skipped = _suites_for_run(cfg, prov, args.suite)
    for s in skipped:
        spec = cfg["suites"][s]
        reason = ("requires elicitation (claude-code only)"
                  if spec.get("requires_elicitation")
                  else "requires oauth-stub server")
        report.event("suite_skipped", provider=args.provider, suite=s,
                     reason=reason)
    failed = []
    for suite in names:
        try:
            if not _run_cli_suite(prov, probe, report, suite):
                failed.append(suite)
        except Exception as e:
            report.failure(TEST_FAILED, f"{args.provider}/{suite}: {e}")
            failed.append(suite)
    path = report.write()
    for line in report.summary_lines():
        print(line)
    print(f"wrote {path}")
    sys.exit(1 if failed else 0)


def _nightly_cli_provider(provider_name, prov, probe, report, cfg) -> bool:
    """One CLI provider inside the nightly: env/binary -> setup -> suites."""
    ok = True
    try:
        prov.check_env()
    except EnvError as e:
        report.failure(KEY_MISSING, f"{provider_name}: {e.var}")
        print(str(e), file=sys.stderr)
        return False
    try:
        prov.ensure_binary()
    except BinaryError as e:
        report.failure(BINARY_MISSING, f"{provider_name}: {e.binary}")
        print(str(e), file=sys.stderr)
        return False
    report.event("env_binary_ready", provider=provider_name)
    if not prov.is_setup():
        report.failure(SETUP_NEEDED, provider_name)
        return False
    names, skipped = _suites_for_run(cfg, prov)
    for s in skipped:
        spec = cfg["suites"][s]
        reason = ("requires elicitation (claude-code only)"
                  if spec.get("requires_elicitation")
                  else "requires oauth-stub server")
        report.event("suite_skipped", provider=provider_name, suite=s,
                     reason=reason)
    for suite in names:
        try:
            if not _run_cli_suite(prov, probe, report, suite):
                ok = False
        except Exception as e:
            report.failure(TEST_FAILED, f"{provider_name}/{suite}: {e}")
            ok = False
    return ok


def _nightly_auth(provider_name: str, cfg: dict, report: Report):
    """check-auth with exactly one magic-link self-heal attempt.

    Returns the logged-in verdict dict, or None when the provider must be
    skipped (failure already recorded in the report). Never loops: one
    heal attempt, one re-check, then give up for this run.
    """
    try:
        return check_auth_or_die(provider_name, cfg)
    except SystemExit as e:
        if e.code != EXIT_AUTH:
            raise
    ml = ((cfg.get("providers") or {}).get(provider_name) or {}).get("magic_link") or {}
    if not ml.get("enabled", False):
        report.failure(AUTH_EXPIRED, provider_name)
        return None
    report.event("auth_heal_attempt", provider=provider_name, via="magic-link")
    print(f"{provider_name}: session expired; one magic-link re-login attempt...",
          file=sys.stderr)
    try:
        magic_link_login(provider_name, cfg)
    except SystemExit as e:
        # magic_link_login exits AUTH_EXPIRED when the flow completes but
        # the session is still not logged in. That must fail just this
        # provider, not abort the whole nightly.
        if e.code == EXIT_AUTH:
            report.failure(AUTH_EXPIRED,
                           f"{provider_name}: magic-link heal finished but "
                           f"session not logged in")
            return None
        raise
    except CodeTimeout as e:
        report.failure(CODE_TIMEOUT, f"{provider_name}: {e}")
        return None
    except MagicLinkConfigError as e:
        report.failure(CONFIG_ERROR, f"{provider_name}: {e}")
        return None
    except MagicLinkError as e:
        report.failure(TEST_FAILED, f"{provider_name}: magic-link heal: {e}")
        return None
    report.event("auth_heal_done", provider=provider_name)
    try:
        return check_auth_or_die(provider_name, cfg)
    except SystemExit as e:
        if e.code != EXIT_AUTH:
            raise
        # Heal ran but the session still isn't valid: record it, don't
        # silently skip the provider.
        report.failure(AUTH_EXPIRED,
                       f"{provider_name}: still expired after magic-link heal")
        return None


def cmd_nightly(args, cfg):
    """check-auth -> setup-verify -> all tests, all providers, one report."""
    report = Report(cfg, "nightly")
    report.command(["nightly"])
    probe = ServerProbe(cfg, skew_allowance=float(cfg["server"].get("skew_allowance", 5.0)))
    try:
        probe.check_reachable()
    except ProbeError as e:
        report.failure(e.signal, e.detail)
        path = report.write()
        print(f"{e.signal}", file=sys.stderr)
        for line in report.summary_lines():
            print(line)
        print(f"wrote {path}")
        sys.exit(e.exit_code)

    overall_ok = True
    for provider_name in NIGHTLY_ORDER:
        prov = get_provider(provider_name, cfg)
        if is_cli_provider(provider_name):
            overall_ok = _nightly_cli_provider(
                provider_name, prov, probe, report, cfg) and overall_ok
            continue
        # check-auth, with one magic-link self-heal attempt when enabled:
        # fail the provider fast, keep going with the others.
        verdict = _nightly_auth(provider_name, cfg, report)
        if verdict is None:
            overall_ok = False
            continue
        report.event("auth_verdict", provider=provider_name, **verdict)
        with persistent_context(provider_name, headless=True) as ctx:
            page = ctx.new_page()
            try:
                setup = prov.setup_probe(page)
            except SystemExit as e:
                # SELECTOR_STALE etc: record, keep the report, next provider.
                report.failure("SETUP_PROBE_FAILED",
                               f"{provider_name}: setup probe exited {e.code}")
                print(f"SETUP_PROBE_FAILED {provider_name} (exit {e.code})",
                      file=sys.stderr)
                overall_ok = False
                continue
            report.event("setup_probe", provider=provider_name, **setup)
            if not setup["ok"]:
                report.failure(SETUP_NEEDED,
                               f"{provider_name}: {setup['detail']}")
                overall_ok = False
                continue
            names, skipped = _suites_for_run(cfg)
            for s in skipped:
                report.event("suite_skipped", provider=provider_name, suite=s,
                             reason="requires oauth-stub server")
            for suite in names:
                try:
                    if not _run_suite(prov, page, probe, report, suite):
                        overall_ok = False
                except SystemExit as e:
                    # TOOL_NOT_CALLED etc: record, continue with next suite.
                    overall_ok = False
                    if e.code not in (1, 7):
                        raise
                except Exception as e:
                    report.failure(TEST_FAILED, f"{provider_name}/{suite}: {e}")
                    overall_ok = False
    path = report.write()
    for line in report.summary_lines():
        print(line)
    print(f"wrote {path}")
    sys.exit(0 if overall_ok else 1)


def cmd_selftest(args, cfg):
    """Review battery: unit tests + every failure signal, no credentials.

    Exercises AUTH_EXPIRED separately via `check-auth` (needs no login to
    fail). Each step prints its signal and exit code; any deviation fails
    the battery.
    """
    import subprocess
    import tempfile

    root = config_mod.HARNESS_ROOT
    py = sys.executable
    failures = []

    def check(name: str, argv: list[str], want_code: int, want_signal: str):
        r = subprocess.run(argv, capture_output=True, text=True, cwd=root)
        sig_ok = want_signal in (r.stderr + r.stdout)
        code_ok = r.returncode == want_code
        status = "PASS" if (sig_ok and code_ok) else "FAIL"
        print(f"[{status}] {name}: exit={r.returncode} (want {want_code}) "
              f"signal={'found' if sig_ok else 'MISSING'}")
        if status == "FAIL":
            failures.append(name)
            print("  stderr:", r.stderr.strip()[:300])

    print("== unit tests ==")
    r = subprocess.run([py, "tests/test_harness.py"], capture_output=True,
                       text=True, cwd=root)
    print(r.stdout.strip().splitlines()[-1] if r.stdout else r.stderr[:200])
    if r.returncode != 0:
        failures.append("unit tests")
        print(r.stdout[-2000:])

    print("== failure signals (no credentials) ==")

    def script_check(name: str, body: str, want_code: int, want_signal: str):
        with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as f:
            f.write("import sys; sys.path.insert(0, '.')\n" + body)
            path = f.name
        try:
            check(name, [py, path], want_code, want_signal)
        finally:
            os.unlink(path)

    # SETUP_NEEDED: the setup-verify signal path with a not-installed probe.
    script_check("SETUP_NEEDED",
        "from harness import SETUP_NEEDED, EXIT_SETUP\n"
        "print(SETUP_NEEDED, file=sys.stderr)\n"
        "print(\"claude: connector 'Maidan Dev' not listed; \"\n"
        "      \"run `harness setup claude`\", file=sys.stderr)\n"
        "sys.exit(EXIT_SETUP)\n",
        4, "SETUP_NEEDED")

    # SELECTOR_STALE: real headless browser on a blank page.
    script_check("SELECTOR_STALE",
        "from playwright.sync_api import sync_playwright\n"
        "from harness.selectors import find\n"
        "with sync_playwright() as p:\n"
        "    b = p.chromium.launch()\n"
        "    pg = b.new_page()\n"
        "    pg.goto('data:text/html,<html><head><title>blank</title></head>'\n"
        "            '<body></body></html>')\n"
        "    find(pg, 'composer', ['div.nonexistent-xyz'], timeout=1000)\n"
        "    b.close()\n",
        5, "SELECTOR_STALE")

    # SERVER_UNREACHABLE: a definitely-closed port.
    script_check("SERVER_UNREACHABLE",
        "from harness import config as c\n"
        "from harness.probe import ServerProbe, ProbeError, die\n"
        "cfg = c.load()\n"
        "cfg['server']['base_url'] = 'http://127.0.0.1:9'\n"
        "try:\n"
        "    ServerProbe(cfg).check_reachable()\n"
        "except ProbeError as e:\n"
        "    die(e)\n",
        6, "SERVER_UNREACHABLE")

    # TOOL_NOT_CALLED: fixture log with no matching tool.
    with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as f:
        f.write("2026-10-07T00:00:00Z INFO unrelated line\n")
        logpath = f.name
    script_check("TOOL_NOT_CALLED",
        "from harness import config as c\n"
        "from harness.probe import ServerProbe, ProbeError, die\n"
        "cfg = c.load()\n"
        f"cfg['server']['log_file'] = {logpath!r}\n"
        "p = ServerProbe(cfg)\n"
        "p.mark()\n"
        "p._pos = 0\n"
        "try:\n"
        "    p.wait_for_tool('post_message', timeout=1, settle_s=0)\n"
        "except ProbeError as e:\n"
        "    die(e)\n",
        7, "TOOL_NOT_CALLED")
    os.unlink(logpath)

    print("== AUTH_EXPIRED: real check-auth with no profile (exit 3) ==")
    # Real code path: headless browser, no saved profile -> the marker
    # state machine must verdict unknown/logged-out -> AUTH_EXPIRED.
    # Slow (~40s: browser launch + 25s marker timeout); the price of
    # exercising the highest-risk seam for real.
    check("AUTH_EXPIRED", [py, "-m", "harness", "check-auth", "claude"],
          3, "AUTH_EXPIRED")

    print("== new signals (no credentials/keys) ==")
    # KEY_MISSING: grok with XAI_API_KEY scrubbed and the vault
    # fallback neutralized.
    script_check("KEY_MISSING",
        "import os\n"
        "from harness import config as c\n"
        "from harness.providers import get_provider\n"
        "from harness.providers.cli_base import EnvError, die_env\n"
        "os.environ.pop('XAI_API_KEY', None)\n"
        "prov = get_provider('grok', c.load())\n"
        "prov._vault_surrogate = lambda: None\n"
        "try:\n"
        "    prov.check_env()\n"
        "except EnvError as e:\n"
        "    die_env(e)\n",
        8, "KEY_MISSING")
    # BINARY_MISSING: provider whose binary can never be on PATH.
    script_check("BINARY_MISSING",
        "from harness import config as c\n"
        "from harness.providers import get_provider\n"
        "from harness.providers.cli_base import BinaryError, die_binary\n"
        "prov = get_provider('gemini', c.load())\n"
        "prov.binary = 'definitely-not-a-real-binary-xyz'\n"
        "prov.npm_package = None\n"
        "try:\n"
        "    prov.ensure_binary()\n"
        "except BinaryError as e:\n"
        "    die_binary(e)\n",
        9, "BINARY_MISSING")
    # MANUAL_ONLY: `harness test cursor` refuses unattended runs.
    check("MANUAL_ONLY", [py, "-m", "harness", "test", "cursor"], 10,
          "MANUAL_ONLY")
    # CONFIG_ERROR: grok install() with a key but a loopback MCP URL refuses
    # with CONFIG_ERROR (exit 11), never a traceback, never KEY_MISSING.
    script_check("CONFIG_ERROR",
        "import os\n"
        "from harness import config as c\n"
        "from harness.providers import get_provider\n"
        "from harness.providers.cli_base import ConfigError, die_config\n"
        "os.environ['XAI_API_KEY'] = 'dummy'\n"
        "prov = get_provider('grok', c.load())\n"
        "try:\n"
        "    prov.install()\n"
        "except ConfigError as e:\n"
        "    die_config(e)\n"
        "print('UNEXPECTED: install did not refuse loopback', file=sys.stderr)\n"
        "sys.exit(1)\n",
        11, "CONFIG_ERROR")
    # CODE_TIMEOUT: the magic-link poller with an impossible recency
    # window. Uses the injected-fetch hook: zero Gmail calls, but the real
    # timeout path (deadline -> CodeTimeout -> exit 13).
    script_check("CODE_TIMEOUT",
        "import time\n"
        "from harness import CODE_TIMEOUT, EXIT_CODE_TIMEOUT\n"
        "from harness.magiclink import poll_for_signin, CodeTimeout\n"
        "import sys\n"
        "try:\n"
        "    poll_for_signin('claude', 'nobody@example.com', time.time(),\n"
        "                    timeout=4, poll_interval=1,\n"
        "                    _fetch=lambda q: [])\n"
        "except CodeTimeout as e:\n"
        "    print(CODE_TIMEOUT, file=sys.stderr)\n"
        "    sys.exit(EXIT_CODE_TIMEOUT)\n"
        "print('UNEXPECTED: poll did not time out', file=sys.stderr)\n"
        "sys.exit(1)\n",
        13, "CODE_TIMEOUT")
    script_check("ELICITATION_GUARD",
        "import sys\n"
        "from harness import config as c\n"
        "from harness.cli import _suites_for_run\n"
        "from harness.providers import get_provider\n"
        "cfg = c.load()\n"
        "cfg['elicitation']['enabled'] = True\n"
        "for name in ('grok', 'gemini', 'copilot-mcp', 'copilot-plugin',\n"
        "             'cursor', 'meta'):\n"
        "    prov = get_provider(name, cfg)\n"
        "    names, skipped = _suites_for_run(cfg, prov, 'elicitation')\n"
        "    assert names == [], f'{name}: unexpectedly runnable: {names}'\n"
        "print('elicitation refused everywhere except claude-code')\n",
        0, "elicitation refused")
    if failures:
        print(f"SELFTEST FAILURES: {failures}")
        sys.exit(1)
    print("SELFTEST: all green")


def cmd_config_sha(args, cfg):
    import re
    path = config_mod.DEFAULT_CONFIG
    with open(path) as f:
        text = f.read()
    text = re.sub(r'commit_sha:.*', f'commit_sha: "{args.sha}"', text)
    with open(path, "w") as f:
        f.write(text)
    print(f"recorded server commit_sha={args.sha}")


def main(argv=None):
    ap = argparse.ArgumentParser(prog="harness",
                                 description="Maidan connected-app E2E harness")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("auth-login", help="one-time manual login (headed)")
    p.add_argument("provider", choices=list(BROWSER_PROVIDERS) + list(CLI_PROVIDERS))
    p.add_argument("--via-email", action="store_true",
                   help="use the headless magic-link flow instead of manual login")
    p.set_defaults(fn=cmd_auth_login)

    p = sub.add_parser("check-auth", help="fail fast unless session/env/binary ready")
    p.add_argument("provider", choices=list(BROWSER_PROVIDERS) + list(CLI_PROVIDERS))
    p.set_defaults(fn=cmd_check_auth)

    p = sub.add_parser("setup", help="install connector/plugin if missing")
    p.add_argument("provider", choices=list(BROWSER_PROVIDERS) + list(CLI_PROVIDERS))
    p.set_defaults(fn=cmd_setup)

    p = sub.add_parser("setup-verify", help="verify install; SETUP_NEEDED if missing")
    p.add_argument("provider", choices=list(BROWSER_PROVIDERS) + list(CLI_PROVIDERS))
    p.set_defaults(fn=cmd_setup_verify)

    p = sub.add_parser("test", help="run test suite(s)")
    p.add_argument("provider", choices=list(BROWSER_PROVIDERS) + list(CLI_PROVIDERS))
    p.add_argument("--suite", default="all",
                   choices=["all", "reads", "multi_tool", "approval", "writes",
                            "elicitation"])
    p.set_defaults(fn=cmd_test)

    p = sub.add_parser("nightly", help="full unattended run, timestamped report")
    p.set_defaults(fn=cmd_nightly)

    p = sub.add_parser("selftest",
                       help="review battery: unit tests + failure signals, no credentials")
    p.set_defaults(fn=cmd_selftest)

    p = sub.add_parser("config-sha", help="record server commit SHA")
    p.add_argument("sha")
    p.set_defaults(fn=cmd_config_sha)

    args = ap.parse_args(argv)
    cfg = config_mod.load()
    args.fn(args, cfg)


if __name__ == "__main__":
    main()
