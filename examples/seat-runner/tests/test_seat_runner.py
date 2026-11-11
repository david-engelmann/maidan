"""The seat runner against a fake Maidan, a fake `capped` and a fake harness."""
import io
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

import seat_runner  # noqa: E402

THREAD = "11111111-1111-1111-1111-111111111111"
LEASE = "22222222-2222-2222-2222-222222222222"
WS = "33333333-3333-3333-3333-333333333333"


class FakeMaidan:
    """Answers queue-depth with `ready` and claim-next with a claim or null,
    and records every call."""

    def __init__(self, ready: int = 1, claim: bool = True, ack_status: int = 200,
                 drop_releases: int = 0, kind: str | None = None, renew_status: int = 200) -> None:
        self.kind = kind
        self.ready = ready
        self.claim = claim
        self.ack_status = ack_status
        self.renew_status = renew_status
        # Hang up on this many release calls before answering one.
        self.drop_releases = drop_releases
        self.on_ack = lambda: None
        self.on_release = lambda: None
        self.calls: list[tuple[str, str, object]] = []
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def answer(self, method):
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(n)) if n else None
                fake.calls.append((method, self.path, body))
                status = 200
                if self.path.endswith("/claim/release"):
                    fake.on_release()
                    if fake.drop_releases > 0:
                        fake.drop_releases -= 1
                        self.close_connection = True
                        return
                if self.path.endswith("/claim/acknowledge"):
                    fake.on_ack()
                    status = fake.ack_status
                if self.path.endswith("/claim/renew"):
                    status = fake.renew_status
                if self.path.startswith(f"/workspaces/{WS}/queue-depth"):
                    out = {"open": fake.ready, "ready": fake.ready, "assigned": 0, "blocked": 0, "unclaimable": 0}
                elif self.path.endswith("/threads/claim-next"):
                    out = {"id": THREAD, "claim_lease_id": LEASE} if fake.claim else None
                    if out is not None and fake.kind:
                        out["attributes"] = {"kind": fake.kind}
                else:
                    out = {}
                data = json.dumps(out).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                self.answer("GET")

            def do_POST(self):
                self.answer("POST")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def paths(self, method: str | None = None) -> list[str]:
        return [p for m, p, _ in self.calls if method in (None, m)]

    def close(self) -> None:
        self.server.shutdown()


class SeatRunnerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        # Outside the checkout (self.tmp), as the runner requires.
        self.state = Path(tempfile.mkdtemp()) / "state"
        self.harness_log = self.tmp / "harness.log"
        os.environ["FAKE_HARNESS_LOG"] = str(self.harness_log)
        os.environ["FAKE_HARNESS_SECS"] = "0"
        os.environ["MAIDAN_TOKEN"] = "seat-token"
        for var in ("FAKE_HARNESS_IGNORE_TERM", "FAKE_HARNESS_EXIT", "ANTHROPIC_BASE_URL",
                    "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ZAI_API_KEY"):
            os.environ.pop(var, None)

    def config(self, maidan: FakeMaidan, **over) -> Path:
        values = {
            "model": "example-model",
            "preflight": '[["true"]]',
            "run_secs": 30,
            "max_load15": 100000,
            "min_free_gb": 0,
            "harness_extra": "",
            "name": "codex",
            "models": '["example-model"]',
            "writable_roots": f'["{self.tmp / "build-slots"}"]',
            **over,
        }
        path = self.tmp / "seats.toml"
        path.write_text(f"""
seats = 1
idle_secs = 0
kill_grace_secs = 1
[maidan]
url = "{maidan.url}"
workspace_id = "{WS}"
release_backoff_secs = 0.01
{f"lease_secs = {values['lease_secs']}" if "lease_secs" in values else ""}
[harness]
name = "{values['name']}"
bin = "{HERE / 'fake_harness.py'}"
model = "{values['model']}"
models = {values['models']}
state_dir = "{self.state}"
{values['harness_extra']}
[caps]
capped = "{HERE / 'fake_capped.py'}"
run_secs = {values['run_secs']}
[host]
max_load15 = {values['max_load15']}
min_free_gb = {values['min_free_gb']}
[[repos]]
repo = "acme/api"
checkout = "{self.tmp}"
preflight = {values['preflight']}
writable_roots = {values['writable_roots']}
""")
        return path

    def runner(self, maidan: FakeMaidan, **over) -> tuple[seat_runner.Runner, io.StringIO]:
        cfg = seat_runner.load_config(self.config(maidan, **over), dict(os.environ))
        out = io.StringIO()
        return seat_runner.Runner(cfg, out=out), out

    def harness_runs(self) -> list[dict]:
        if not self.harness_log.exists():
            return []
        return [json.loads(line) for line in self.harness_log.read_text().splitlines()]

    def test_a_failed_preflight_claims_nothing(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan, preflight='[["true"], ["false"]]')
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "preflight_failed")
        self.assertFalse(any("claim" in p for p in maidan.paths()), maidan.calls)
        self.assertEqual(self.harness_runs(), [])
        self.assertEqual(len(out.getvalue().splitlines()), 1, "one log line per run")

    def test_a_harness_timeout_releases_the_claim(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        os.environ["FAKE_HARNESS_SECS"] = "30"
        runner, out = self.runner(maidan, run_secs=1)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "timed_out")
        release = [b for m, p, b in maidan.calls if p == f"/threads/{THREAD}/claim/release"]
        self.assertEqual(release, [{"claim_lease_id": LEASE}])
        line = json.loads(out.getvalue())
        self.assertEqual((line["outcome"], line["exit"], line["thread"]), ("timed_out", 124, THREAD))

    def test_a_null_claim_exits_zero_without_running_the_harness(self):
        maidan = FakeMaidan(claim=False)
        self.addCleanup(maidan.close)
        code = seat_runner.main(["--config", str(self.config(maidan)), "--once"])
        self.assertEqual(code, 0)
        self.assertEqual(self.harness_runs(), [])
        self.assertIn(f"/workspaces/{WS}/threads/claim-next", maidan.paths("POST"))

    def test_an_empty_queue_for_the_seats_engine_and_repo_claims_nothing(self):
        maidan = FakeMaidan(ready=0)
        self.addCleanup(maidan.close)
        runner, _ = self.runner(maidan)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "no_work")
        self.assertEqual(maidan.paths(), [f"/workspaces/{WS}/queue-depth?engine=session&repo=acme%2Fapi"])

    def test_a_busy_host_runs_neither_the_preflight_nor_a_claim(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan, max_load15=0)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "host_busy")
        self.assertEqual(maidan.calls, [])
        self.assertIn("15-min load", out.getvalue())

    def test_a_model_outside_the_harness_allowlist_is_refused_at_startup(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        with self.assertRaises(seat_runner.ConfigError):
            seat_runner.load_config(self.config(maidan, model="other-model"), dict(os.environ))
        self.assertEqual(seat_runner.main(["--config", str(self.config(maidan, model="other-model"))]), 2)
        self.assertEqual(maidan.calls, [])

    def test_the_harness_runs_with_the_model_approval_mode_writable_roots_and_the_claim(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        runner, _ = self.runner(maidan)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        (run,) = self.harness_runs()
        argv = run["argv"]
        self.assertEqual(argv[:3], ["exec", "-m", "example-model"])
        self.assertIn('mcp_servers.maidan.default_tools_approval_mode="approve"', argv)
        self.assertIn(f'sandbox_workspace_write.writable_roots=["{self.tmp / "build-slots"}"]', argv)
        self.assertIn(THREAD, argv[-1])
        self.assertIn(LEASE, argv[-1])
        self.assertNotIn("seat-token", " ".join(argv), "the token is never on the command line")
        posts = maidan.paths("POST")
        self.assertEqual(
            posts,
            [
                f"/workspaces/{WS}/threads/claim-next",
                f"/threads/{THREAD}/claim/acknowledge",
                f"/threads/{THREAD}/claim/release",
            ],
        )

    def test_sigterm_kills_the_harness_and_releases_its_claim(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        os.environ["FAKE_HARNESS_SECS"] = "60"
        proc = subprocess.Popen(
            [sys.executable, str(HERE.parent / "seat_runner.py"), "--config", str(self.config(maidan, run_secs=120))],
            stdout=subprocess.PIPE,
            text=True,
        )
        deadline = time.monotonic() + 20
        while not self.harness_runs() and time.monotonic() < deadline:
            time.sleep(0.1)
        (run,) = self.harness_runs()
        proc.send_signal(signal.SIGTERM)
        out, _ = proc.communicate(timeout=20)
        self.assertEqual(proc.returncode, 0)
        self.assertIn(f"/threads/{THREAD}/claim/release", maidan.paths("POST"))
        self.assertEqual(json.loads(out.splitlines()[-1])["outcome"], "stopped")
        time.sleep(0.5)
        with self.assertRaises(ProcessLookupError, msg="the harness is gone"):
            os.kill(run["pid"], 0)

    def test_a_long_harness_renews_the_lease_and_stops_before_release(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        os.environ["FAKE_HARNESS_SECS"] = "1"
        runner, _ = self.runner(maidan, lease_secs=1, run_secs=30)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        paths = maidan.paths("POST")
        renews = [i for i, path in enumerate(paths) if path.endswith("/claim/renew")]
        release = next(i for i, path in enumerate(paths) if path.endswith("/claim/release"))
        self.assertTrue(renews, paths)
        self.assertLess(renews[-1], release)

    def test_a_refused_renewal_lets_the_harness_finish(self):
        maidan = FakeMaidan(renew_status=409)
        self.addCleanup(maidan.close)
        os.environ["FAKE_HARNESS_SECS"] = "1"
        runner, _ = self.runner(maidan, lease_secs=1, run_secs=30)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        self.assertEqual(len(self.harness_runs()), 1)
        self.assertTrue(any(path.endswith("/claim/renew") for path in maidan.paths("POST")))
        self.assertEqual(self.releases(maidan), [{"claim_lease_id": LEASE}])

    def releases(self, maidan: FakeMaidan) -> list[object]:
        return [b for _, p, b in maidan.calls if p == f"/threads/{THREAD}/claim/release"]

    def test_an_unreachable_release_is_retried_until_the_server_answers(self):
        maidan = FakeMaidan(drop_releases=2)
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        self.assertEqual(len(self.releases(maidan)), 3)
        self.assertEqual(json.loads(out.getvalue())["release"], 200)

    def test_a_seat_survives_a_server_that_is_down(self):
        maidan = FakeMaidan()
        runner, out = self.runner(maidan)
        maidan.close()
        maidan.server.server_close()
        runner.seat_loop(0, once=True)
        self.assertEqual(json.loads(out.getvalue())["outcome"], "queue_unreachable")

    def test_a_seat_logs_an_unexpected_error_and_keeps_its_thread(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan)
        runs = []

        def boom(seat, repo):
            runs.append(seat)
            if len(runs) == 1:
                raise RuntimeError("boom")
            runner.stop.set()
            return "no_work"

        runner.run_once = boom
        runner.seat_loop(0, once=False)
        self.assertEqual(len(runs), 2, "the loop went on after the error")
        line = json.loads(out.getvalue().splitlines()[0])
        self.assertEqual((line["outcome"], line["detail"]), ("error", "RuntimeError: boom"))

    def test_a_stop_between_claim_and_launch_releases_without_starting_the_harness(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan)
        maidan.on_ack = runner.shutdown
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "stopped")
        self.assertEqual(self.harness_runs(), [])
        self.assertEqual(self.releases(maidan), [{"claim_lease_id": LEASE}])

    def test_a_failed_acknowledge_releases_without_starting_the_harness(self):
        maidan = FakeMaidan(ack_status=409)
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ack_failed")
        self.assertEqual(self.harness_runs(), [])
        self.assertEqual(self.releases(maidan), [{"claim_lease_id": LEASE}])

    def test_a_harness_that_ignores_sigterm_is_killed_before_its_claim_is_released(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        os.environ["FAKE_HARNESS_SECS"] = "60"
        os.environ["FAKE_HARNESS_IGNORE_TERM"] = "1"
        runner, _ = self.runner(maidan, run_secs=120)
        alive_at_release = []

        def check():
            (run,) = self.harness_runs()
            try:
                os.kill(run["pid"], 0)
                alive_at_release.append(True)
            except ProcessLookupError:
                alive_at_release.append(False)

        maidan.on_release = check
        seat = threading.Thread(target=runner.run_once, args=(0, runner.cfg.repos[0]))
        seat.start()
        deadline = time.monotonic() + 20
        while not self.harness_runs() and time.monotonic() < deadline:
            time.sleep(0.1)
        time.sleep(0.5)
        runner.shutdown()
        seat.join(30)
        self.assertFalse(seat.is_alive())
        self.assertEqual(alive_at_release, [False])

    def test_the_preflight_never_sees_the_seat_token_or_other_listed_secrets(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        os.environ["OTHER_SECRET"] = "hush"
        self.addCleanup(os.environ.pop, "OTHER_SECRET", None)
        dump = self.tmp / "preflight.env"
        path = self.config(maidan, preflight=f'[["sh", "-c", "env > {dump}"]]')
        path.write_text(path.read_text().replace(
            "[maidan]\n", '[maidan]\nsecret_env = ["OTHER_SECRET"]\n'))
        runner = seat_runner.Runner(seat_runner.load_config(path, dict(os.environ)), out=io.StringIO())
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        env = dump.read_text()
        self.assertNotIn("seat-token", env)
        self.assertNotIn("hush", env)
        (run,) = self.harness_runs()
        self.assertEqual(run["token"], "seat-token", "the harness gets it for its MCP connection")

    def test_the_harness_runs_without_the_token_when_pass_token_is_off(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        path = self.config(maidan)
        path.write_text(path.read_text().replace("[harness]\n", "[harness]\npass_token = false\n"))
        runner = seat_runner.Runner(seat_runner.load_config(path, dict(os.environ)), out=io.StringIO())
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        (run,) = self.harness_runs()
        self.assertIsNone(run["token"])

    def test_a_pid_outside_the_tree_survives_a_tree_kill(self):
        outsider = subprocess.Popen(["sleep", "60"])
        self.addCleanup(outsider.kill)
        child = subprocess.Popen(["sleep", "60"], start_new_session=True)
        self.addCleanup(child.kill)
        tree = seat_runner.Tree(child.pid)
        tree.refresh()
        tree.pids[outsider.pid] = child.pid  # as if a tracked pid had been reused
        tree.kill(1)
        self.assertEqual(child.wait(10), -signal.SIGTERM)
        self.assertIsNone(outsider.poll(), "the outside process is still running")

    def test_a_process_holding_a_reaped_roots_pid_survives_a_tree_kill(self):
        outsider = subprocess.Popen(["sleep", "60"])
        self.addCleanup(outsider.kill)
        tree = seat_runner.Tree(outsider.pid)  # the reaped root's pid, reused
        tree.groups = set()
        tree.root_reaped()
        tree.kill(1)
        self.assertIsNone(outsider.poll(), "the process now holding the pid is still running")

    def test_a_model_list_missing_from_the_config_is_refused_at_startup(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        path = self.config(maidan)
        path.write_text(path.read_text().replace('models = ["example-model"]\n', ""))
        with self.assertRaises(seat_runner.ConfigError):
            seat_runner.load_config(path, dict(os.environ))

    def test_a_review_thread_runs_the_review_prompt_and_other_kinds_the_default(self):
        review = self.tmp / "review-prompt.md"
        review.write_text("REVIEW-PROMPT-MARKER: red-team this pull request.")
        extra = f'prompts = {{ review = "{review}" }}'
        for kind, marker, skips_runbook in [("review", "REVIEW-PROMPT-MARKER", False), ("change", "session claimant", True)]:
            self.harness_log.unlink(missing_ok=True)
            maidan = FakeMaidan(kind=kind)
            self.addCleanup(maidan.close)
            runner, _ = self.runner(maidan, harness_extra=extra)
            self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
            (run,) = self.harness_runs()
            prompt = run["argv"][-1]
            self.assertIn(marker, prompt, kind)
            self.assertIn(THREAD, prompt)
            self.assertEqual("start at \"3. Read the task\"" in prompt, skips_runbook, kind)

    def test_a_kind_prompt_that_does_not_exist_is_refused_at_startup(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        with self.assertRaises(seat_runner.ConfigError):
            self.runner(maidan, harness_extra=f'prompts = {{ review = "{self.tmp / "missing.md"}" }}')

    # ------------------------------------------------------------ harnesses

    GLM = ('provider = "z.ai"\n'
           'env = { ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic", API_TIMEOUT_MS = "3000000" }\n'
           'env_from = { ANTHROPIC_AUTH_TOKEN = "ZAI_API_KEY" }')

    def run_harness(self, name: str, model: str = "example-model", extra: str = "", **over) -> dict:
        maidan = FakeMaidan(**over.pop("maidan", {}))
        self.addCleanup(maidan.close)
        runner, out = self.runner(maidan, name=name, model=model, models=f'["{model}"]', harness_extra=extra, **over)
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        (run,) = self.harness_runs()
        run["maidan_url"] = maidan.url
        run["out"] = out.getvalue()
        return run

    def assert_token_nowhere(self, run: dict) -> None:
        """Not on the harness's command line, not in the run's log line, and
        in no file the run left (the fake's own log records it on purpose)."""
        self.assertNotIn("seat-token", json.dumps(run["argv"]))
        self.assertNotIn("seat-token", run["out"])
        for path in [*self.tmp.rglob("*"), *self.state.parent.rglob("*")]:
            if path.is_file() and path != self.harness_log:
                self.assertNotIn("seat-token", path.read_text(errors="replace"), path)

    def assert_no_run_files(self) -> None:
        left = list(self.state.iterdir()) if self.state.exists() else []
        self.assertEqual(left, [], "the run's private directory is gone")

    def test_codex_runs_exactly_the_command_it_did_before(self):
        run = self.run_harness("codex")
        prompt = run["argv"][-1]
        self.assertEqual(run["argv"], [
            "exec", "-m", "example-model", "--sandbox", "workspace-write", "--ephemeral",
            "-c", 'mcp_servers.maidan.default_tools_approval_mode="approve"',
            "-c", f'sandbox_workspace_write.writable_roots=["{self.tmp / "build-slots"}"]',
            prompt,
        ])
        self.assertIn('evidence: {"provider": "openai", "harness": "codex"}', prompt)
        self.assertIn("`~/.codex/maidan-claimant`", prompt)
        self.assertEqual(run["token"], "seat-token")
        self.assertIsNone(run["settings"], "codex reads its own config.toml")
        self.assert_token_nowhere(run)

    def test_claude_runs_headless_with_maidan_inline_and_the_token_read_from_its_variable(self):
        run = self.run_harness("claude", model="claude-sonnet-5")
        argv = run["argv"]
        self.assertEqual(argv[0], "-p")
        self.assertIn("session claimant", argv[1], "the prompt comes straight after -p")

        def value(flag):
            return argv[argv.index(flag) + 1]

        self.assertEqual(value("--model"), "claude-sonnet-5")
        self.assertEqual(value("--permission-mode"), "acceptEdits")
        self.assertEqual(value("--allowedTools"), "mcp__maidan__*")
        self.assertIn("--strict-mcp-config", argv)
        self.assertIn("--no-session-persistence", argv)
        self.assertEqual(argv[-2:], ["--add-dir", str(self.tmp / "build-slots")])
        server = json.loads(value("--mcp-config"))["mcpServers"]["maidan"]
        self.assertEqual((server["type"], server["url"]), ("http", run["maidan_url"] + "/mcp/streamable"))
        self.assertNotIn("headers", server)
        # The helper, run the way Claude Code runs it (a shell, this seat's
        # environment), prints the header from the variable.
        helper = subprocess.run(["sh", "-c", server["headersHelper"]], capture_output=True, text=True,
                                env={**os.environ, "MAIDAN_TOKEN": "seat-token"}, check=True)
        self.assertEqual(json.loads(helper.stdout), {"Authorization": "Bearer seat-token"})
        sandbox = json.loads(value("--settings"))["sandbox"]
        self.assertEqual((sandbox["enabled"], sandbox["allowUnsandboxedCommands"], sandbox["failIfUnavailable"]),
                         (True, False, True))
        self.assertEqual(sandbox["network"]["allowedDomains"], ["127.0.0.1"])
        self.assertEqual(run["token"], "seat-token", "the helper reads it from the harness's environment")
        self.assertIn('evidence: {"provider": "anthropic", "harness": "claude-code"}', argv[1])
        self.assertIsNone(run["settings"])
        self.assert_token_nowhere(run)
        self.assert_no_run_files()

    def test_gemini_runs_headless_with_a_private_settings_file_that_names_only_the_variable(self):
        run = self.run_harness("gemini", model="gemini-3-pro")
        argv = run["argv"]
        self.assertEqual(argv[0], "-p")
        self.assertEqual(argv[2:], [
            "-m", "gemini-3-pro", "--approval-mode", "yolo", "--sandbox",
            "--allowed-mcp-server-names", "maidan",
            "--include-directories", str(self.tmp / "build-slots"),
        ])
        settings = run["settings"]
        self.assertEqual((settings["mode"], settings["dir_mode"]), (0o600, 0o700))
        server = json.loads(settings["text"])["mcpServers"]["maidan"]
        self.assertEqual(server["httpUrl"], run["maidan_url"] + "/mcp/streamable")
        self.assertEqual(server["headers"], {"Authorization": "Bearer ${MAIDAN_TOKEN}"})
        self.assertTrue(server["trust"])
        self.assertEqual(run["env"]["GEMINI_CLI_TRUST_WORKSPACE"], "true")
        self.assertEqual(run["token"], "seat-token", "Gemini expands ${MAIDAN_TOKEN} from its environment")
        self.assertIn('evidence: {"provider": "google", "harness": "gemini-cli"}', argv[1])
        self.assertFalse(Path(settings["path"]).exists(), "removed after the run")
        self.assert_token_nowhere(run)
        self.assert_no_run_files()

    ZCODE = 'api_key_env = "ZAI_API_KEY"'
    # The harnesses the runner writes private files for: name, model, extra.
    FILE_SEATS = [("gemini", "gemini-3-pro", ""), ("zcode", "glm-5.3", ZCODE)]

    def file_seat(self, maidan: FakeMaidan, name: str, model: str, extra: str, **over):
        os.environ["ZAI_API_KEY"] = "zai-key"
        roots = "[]" if name == "zcode" else f'["{self.tmp / "build-slots"}"]'
        return dict(name=name, model=model, models=f'["{model}"]', harness_extra=extra,
                    writable_roots=roots, **over)

    @staticmethod
    def handed_files(run: dict) -> list[Path]:
        if run["settings"]:
            return [Path(run["settings"]["path"])]
        return [Path(run["zcode"]["provider"]["path"]), Path(run["zcode"]["runtime"]["path"])]

    def test_the_private_files_are_removed_after_the_harness_crashes(self):
        os.environ["FAKE_HARNESS_EXIT"] = "3"
        for name, model, extra in self.FILE_SEATS:
            with self.subTest(name):
                self.harness_log.unlink(missing_ok=True)
                maidan = FakeMaidan()
                self.addCleanup(maidan.close)
                runner, _ = self.runner(maidan, **self.file_seat(maidan, name, model, extra))
                self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "failed")
                (run,) = self.harness_runs()
                for path in self.handed_files(run):
                    self.assertFalse(path.exists(), path)
                self.assert_no_run_files()
                self.assertEqual(self.releases(maidan), [{"claim_lease_id": LEASE}])

    def test_the_private_files_are_removed_when_the_runner_is_stopped(self):
        os.environ["FAKE_HARNESS_SECS"] = "60"
        for name, model, extra in self.FILE_SEATS:
            with self.subTest(name):
                self.harness_log.unlink(missing_ok=True)
                maidan = FakeMaidan()
                self.addCleanup(maidan.close)
                runner, _ = self.runner(maidan, **self.file_seat(maidan, name, model, extra, run_secs=120))
                seat = threading.Thread(target=runner.run_once, args=(0, runner.cfg.repos[0]))
                seat.start()
                deadline = time.monotonic() + 20
                while not self.harness_runs() and time.monotonic() < deadline:
                    time.sleep(0.1)
                (run,) = self.harness_runs()
                for path in self.handed_files(run):
                    self.assertTrue(path.exists(), "there while the harness runs")
                runner.shutdown()
                seat.join(30)
                self.assertFalse(seat.is_alive())
                for path in self.handed_files(run):
                    self.assertFalse(path.exists(), path)
                self.assert_no_run_files()

    def test_a_killed_runners_private_files_are_swept_by_the_next_runner(self):
        os.environ["FAKE_HARNESS_SECS"] = "60"
        for name, model, extra in self.FILE_SEATS:
            with self.subTest(name):
                self.harness_log.unlink(missing_ok=True)
                maidan = FakeMaidan()
                self.addCleanup(maidan.close)
                config = self.config(maidan, **self.file_seat(maidan, name, model, extra, run_secs=120))
                proc = subprocess.Popen([sys.executable, str(HERE.parent / "seat_runner.py"), "--config", str(config)],
                                        stdout=subprocess.DEVNULL)
                deadline = time.monotonic() + 20
                while not self.harness_runs() and time.monotonic() < deadline:
                    time.sleep(0.1)
                (run,) = self.harness_runs()
                proc.kill()
                proc.wait(10)
                os.kill(run["pid"], signal.SIGKILL)
                files = self.handed_files(run)
                self.assertTrue(all(path.exists() for path in files), "a SIGKILLed runner cannot remove them")
                maidan.ready = 0
                self.assertEqual(seat_runner.main(["--config", str(config), "--once"]), 0)
                for path in files:
                    self.assertFalse(path.exists(), "the next runner sweeps a dead runner's directory")
                self.assert_no_run_files()

    def test_zcode_runs_guarded_with_its_model_and_maidan_in_a_private_home(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        dump = self.tmp / "preflight.env"
        if "CARGO_HOME" in os.environ:
            self.addCleanup(os.environ.__setitem__, "CARGO_HOME", os.environ.pop("CARGO_HOME"))
        runner, out = self.runner(maidan, **self.file_seat(maidan, "zcode", "glm-5.3", self.ZCODE,
                                                           preflight=f'[["sh", "-c", "env > {dump}"]]'))
        self.assertEqual((runner.cfg.provider, runner.cfg.family), ("z.ai", "zhipu"))
        self.assertEqual(runner.run_once(0, runner.cfg.repos[0]), "ran")
        (run,) = self.harness_runs()
        run["out"] = out.getvalue()
        argv = run["argv"]
        self.assertEqual(argv[0:2], ["--prompt", argv[1]])
        self.assertEqual(argv[2:], [
            "--cwd", str(self.tmp.resolve()), "--mode", "guarded",
            "--disallowed-tools", "AskUserQuestion EnterPlanMode ExitPlanMode", "--no-color",
        ])
        self.assertNotIn("--model", argv, "zcode has no model flag; the provider file names it")
        files = run["zcode"]
        for handed in (files["provider"], files["runtime"]):
            self.assertEqual(handed["mode"], 0o600, handed["path"])
            self.assertEqual(set(handed["dir_modes"]), {0o700}, handed["path"])
            self.assertFalse(Path(handed["path"]).resolve().is_relative_to(self.tmp.resolve()),
                             "never in the checkout")
        provider = json.loads(files["provider"]["text"])["config"]
        rule = provider["providerConfigRules"]["providerRules"][0]["config"]
        self.assertEqual(rule["access"], {"type": "api-key", "apiKey": "zai-key"})
        self.assertEqual(rule["api"], {"type": "anthropic-messages", "baseUrl": "https://api.z.ai/api/anthropic"})
        self.assertEqual(provider["defaultModelSelection"]["modelId"], "glm-5.3")
        runtime = json.loads(files["runtime"]["text"])
        server = runtime["mcp"]["servers"]["maidan"]
        self.assertTrue(runtime["features"]["mcp"])
        self.assertEqual((server["type"], server["url"]), ("http", maidan.url + "/mcp/streamable"))
        self.assertEqual(server["headers"], {"Authorization": "Bearer seat-token"},
                         "ZCode expands no variables, so the 0600 file holds it for the run")
        env = run["env"]
        self.assertEqual(Path(env["HOME"]), Path(files["runtime"]["path"]).parents[2])
        self.assertEqual(env["ZCODE_DATA_BASE_DIR"], env["HOME"])
        self.assertEqual(env["CARGO_HOME"], os.path.expanduser("~/.cargo"), "the toolchain stays on the real home")
        self.assertNotIn("ZAI_API_KEY", env, "the key is in the provider file only")
        self.assertNotIn("zai-key", dump.read_text(), "the preflight never sees the key")
        self.assertIn('evidence: {"provider": "z.ai", "harness": "zcode"}', argv[1])
        self.assertIn(f"`{os.path.expanduser('~/.zcode/maidan-claimant')}`", argv[1],
                      "the runbook's directory under the real home, not the run's")
        for path in self.handed_files(run):
            self.assertFalse(path.exists())
        self.assertNotIn("zai-key", json.dumps(argv))
        self.assert_token_nowhere(run)
        self.assert_no_run_files()

    def test_a_zcode_seat_that_misstates_its_endpoint_or_key_is_refused(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        refused = {
            "an Anthropic endpoint under z.ai": (
                f'{self.ZCODE}\napi_base_url = "https://api.anthropic.com"', "[]"),
            "no key variable": ("", "[]"),
            "a key variable that is not set": ('api_key_env = "NOT_SET_ANYWHERE"', "[]"),
            "a plain-http endpoint": (f'{self.ZCODE}\napi_base_url = "http://api.z.ai/api/anthropic"', "[]"),
            "an unknown api_type": (f'{self.ZCODE}\napi_type = "grpc"', "[]"),
            "writable_roots it cannot honour": (self.ZCODE, f'["{self.tmp}"]'),
        }
        os.environ["ZAI_API_KEY"] = "zai-key"
        for why, (extra, roots) in refused.items():
            with self.subTest(why), self.assertRaises(seat_runner.ConfigError):
                self.runner(maidan, name="zcode", model="glm-5.3", models='["glm-5.3"]', harness_extra=extra,
                            writable_roots=roots)
        with self.assertRaises(seat_runner.ConfigError, msg="api_key_env is zcode's"):
            self.runner(maidan, harness_extra=self.ZCODE)
        path = self.config(maidan, name="zcode", model="glm-5.3", models='["glm-5.3"]',
                           harness_extra=self.ZCODE, writable_roots="[]")
        path.write_text(path.read_text().replace(f'state_dir = "{self.state}"',
                                                 f'state_dir = "{self.tmp / "zstate"}"'))
        with self.assertRaises(seat_runner.ConfigError, msg="a state_dir inside the checkout"):
            seat_runner.load_config(path, dict(os.environ))

    def test_a_glm_seat_reports_zai_and_gets_only_its_own_key(self):
        os.environ.update({"ZAI_API_KEY": "zai-key", "ANTHROPIC_API_KEY": "anthropic-key"})
        dump = self.tmp / "preflight.env"
        run = self.run_harness("claude", model="glm-5.3", extra=self.GLM,
                               preflight=f'[["sh", "-c", "env > {dump}"]]')
        anthropic = {k: v for k, v in run["env"].items() if k.startswith(("ANTHROPIC_", "ZAI_"))}
        self.assertEqual(anthropic, {
            "ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic",
            "ANTHROPIC_AUTH_TOKEN": "zai-key",
        }, "the GLM key under Claude Code's name; no Anthropic key goes to z.ai")
        self.assertIn('evidence: {"provider": "z.ai", "harness": "claude-code"}', run["argv"][1])
        self.assertIn("(zhipu family)", run["argv"][1])
        self.assertNotIn("zai-key", dump.read_text(), "the preflight never sees the GLM key")
        self.assertNotIn("zai-key", json.dumps(run["argv"]))

    def test_claude_pointed_away_from_anthropic_must_not_say_anthropic(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        refused = {
            "a GLM endpoint left as anthropic":
                'env = { ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic" }',
            "a declared family Maidan would not read":
                'provider = "z.ai"\nfamily = "anthropic"\n'
                'env = { ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic" }',
            "a GLM provider that still talks to Anthropic": 'provider = "z.ai"',
            "a key written in the file":
                'provider = "z.ai"\n'
                'env = { ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic", ANTHROPIC_AUTH_TOKEN = "k" }',
            "a key variable that is not set":
                'provider = "z.ai"\n'
                'env = { ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic" }\n'
                'env_from = { ANTHROPIC_AUTH_TOKEN = "ZAI_API_KEY" }',
        }
        for why, extra in refused.items():
            with self.subTest(why), self.assertRaises(seat_runner.ConfigError):
                self.runner(maidan, name="claude", model="glm-5.3", models='["glm-5.3"]', harness_extra=extra)
        os.environ["ANTHROPIC_BASE_URL"] = "https://api.z.ai/api/anthropic"
        with self.assertRaises(seat_runner.ConfigError, msg="an inherited endpoint counts too"):
            self.runner(maidan, name="claude", model="claude-sonnet-5", models='["claude-sonnet-5"]')
        os.environ["ANTHROPIC_BASE_URL"] = "https://api.anthropic.com"
        runner, _ = self.runner(maidan, name="claude", model="claude-sonnet-5", models='["claude-sonnet-5"]')
        self.assertEqual(runner.cfg.family, "anthropic")
        self.assertEqual(maidan.calls, [])

    def test_a_family_other_than_the_one_maidan_reads_is_refused(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        for name, model, extra in [("gemini", "gemini-3-pro", 'family = "openai"'),
                                   ("codex", "example-model", 'provider = "nobody"'),
                                   ("cursor", "example-model", "")]:
            with self.subTest(name=name, extra=extra), self.assertRaises(seat_runner.ConfigError):
                self.runner(maidan, name=name, model=model, models=f'["{model}"]', harness_extra=extra)

    def test_each_prompt_renders_with_its_own_seats_provider_and_family(self):
        os.environ["ZAI_API_KEY"] = "zai-key"
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        review = HERE.parent / "review-prompt.md"
        seats = [("codex", "example-model", "", "openai", "codex", "openai"),
                 ("claude", "claude-sonnet-5", "", "anthropic", "claude-code", "anthropic"),
                 ("claude", "glm-5.3", self.GLM, "z.ai", "claude-code", "zhipu"),
                 ("gemini", "gemini-3-pro", "", "google", "gemini-cli", "google"),
                 ("zcode", "glm-5.3", self.ZCODE, "z.ai", "zcode", "zhipu")]
        for name, model, extra, provider, harness, family in seats:
            with self.subTest(model=model, harness=harness):
                runner, _ = self.runner(maidan, name=name, model=model, models=f'["{model}"]',
                                        writable_roots="[]",
                                        harness_extra=f'{extra}\nprompts = {{ review = "{review}" }}')
                repo = runner.cfg.repos[0]
                change = seat_runner.run_prompt(runner.cfg, repo, THREAD, LEASE, None)
                verdict = seat_runner.run_prompt(runner.cfg, repo, THREAD, LEASE, "review")
                evidence = f'evidence: {{"provider": "{provider}", "harness": "{harness}"}}'
                for prompt in (change, verdict):
                    self.assertIn(evidence, prompt)
                    self.assertIn(f'`model: "{model}"`', prompt)
                    self.assertNotRegex(prompt, r"\{(provider|harness|family|model|claimant_dir)\}")
                self.assertIn(f"`author_family` is `{family}`, your own family", verdict)
                self.assertIn(f"Red-team by {harness} ({model}, {family} family)", verdict)
                self.assertIn(f'`served: {{"provider": "{provider}", "model": "{model}"}}`', verdict)
                others = {"openai", "anthropic", "google", "zhipu"} - {family}
                for other in others:
                    self.assertNotIn(f"`{other}`", verdict)

    def test_the_family_table_is_the_one_maidan_reads(self):
        source = HERE.parents[2] / "crates" / "maidan-types" / "src" / "task_attributes.rs"
        if not source.exists():
            self.skipTest("not in a Maidan checkout")
        text = source.read_text()
        table = text[text.index("pub const PROVIDER_FAMILIES"):]
        table = table[:table.index("];")]
        rows = re.findall(r'\("(\w+)", &\[([^\]]*)\], &\[([^\]]*)\]\)', table)

        def words(listed):
            return tuple(re.findall(r'"([^"]+)"', listed))

        self.assertEqual(tuple((f, words(w), words(p)) for f, w, p in rows), seat_runner.PROVIDER_FAMILIES)
        self.assertEqual(seat_runner.provider_family("z.ai", "glm-5.3"), "zhipu")
        self.assertEqual(seat_runner.provider_family("z.ai", "claude-sonnet-5"), "anthropic",
                         "why a GLM seat names a glm model, not a Claude alias")


if __name__ == "__main__":
    unittest.main()
