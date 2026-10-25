"""The seat runner against a fake Maidan, a fake `capped` and a fake harness."""
import io
import json
import os
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
                 drop_releases: int = 0) -> None:
        self.ready = ready
        self.claim = claim
        self.ack_status = ack_status
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
                if self.path.startswith(f"/workspaces/{WS}/queue-depth"):
                    out = {"open": fake.ready, "ready": fake.ready, "assigned": 0, "blocked": 0, "unclaimable": 0}
                elif self.path.endswith("/threads/claim-next"):
                    out = {"id": THREAD, "claim_lease_id": LEASE} if fake.claim else None
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
        self.harness_log = self.tmp / "harness.log"
        os.environ["FAKE_HARNESS_LOG"] = str(self.harness_log)
        os.environ["FAKE_HARNESS_SECS"] = "0"
        os.environ["MAIDAN_TOKEN"] = "seat-token"
        os.environ.pop("FAKE_HARNESS_IGNORE_TERM", None)

    def config(self, maidan: FakeMaidan, **over) -> Path:
        values = {
            "model": "example-model",
            "preflight": '[["true"]]',
            "run_secs": 30,
            "max_load15": 100000,
            "min_free_gb": 0,
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
[harness]
name = "codex"
bin = "{HERE / 'fake_harness.py'}"
model = "{values['model']}"
models = ["example-model"]
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
writable_roots = ["{self.tmp / 'build-slots'}"]
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
        tree.pids.add(outsider.pid)  # as if a tracked pid had been reused
        tree.kill(1)
        self.assertEqual(child.wait(10), -signal.SIGTERM)
        self.assertIsNone(outsider.poll(), "the outside process is still running")

    def test_a_model_list_missing_from_the_config_is_refused_at_startup(self):
        maidan = FakeMaidan()
        self.addCleanup(maidan.close)
        path = self.config(maidan)
        path.write_text(path.read_text().replace('models = ["example-model"]\n', ""))
        with self.assertRaises(seat_runner.ConfigError):
            seat_runner.load_config(path, dict(os.environ))


if __name__ == "__main__":
    unittest.main()
