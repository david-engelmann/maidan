#!/usr/bin/env python3
"""The Python claimants under MCP and HTTP faults. Run: python3 scripts/test-mcp-fault-injection.py

Realistic MCP faults cut agent task completion sharply, and a larger model
does not win it back, so the claimant code has to. Each test puts
`scripts/mcp-fault-proxy.py` between a claimant and a fake Maidan and injects
one fault: latency, a timeout, a dropped connection, a 500/502/503 with and
without Retry-After, an answer cut short, a JSON-RPC error, or a result whose
schema drifted. The claimant must either recover within its retry budget, or
stop cleanly: release its claim or never take one, never claim twice or
write a result twice, never loop hot, and log one clear line.

Three claimants: the seat runner (`examples/seat-runner`), a claim loop on
the Python SDK (`sdk/python`), and the OpenRouter claimant's MCP client
(`examples/openrouter/claimant.py`), the one Python claimant that claims over
`/mcp/streamable`. The seat runner and the SDK speak REST; the proxy fronts
both kinds of route.

The fake Maidan keeps the server's Idempotency-Key contract
(crates/maidan-server/src/idempotency.rs): a write repeated with its key gets
the first answer back, so a retry after a lost answer is safe only when the
claimant reuses the key. Standard library only, so it runs anywhere CI has a
python3.
"""

from __future__ import annotations

import argparse
import contextlib
import subprocess
import importlib.util
import io
import json
import os
import sys
import tempfile
import threading
import time
import unittest
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SEAT_DIR = ROOT / "examples" / "seat-runner"


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


faults = _load("mcp_fault_proxy", Path(__file__).with_name("mcp-fault-proxy.py"))
seat_runner = _load("seat_runner", SEAT_DIR / "seat_runner.py")
claimant = _load("openrouter_claimant", ROOT / "examples" / "openrouter" / "claimant.py")
sys.path.insert(0, str(ROOT / "sdk" / "python" / "src"))
import maidan  # noqa: E402

WS = "33333333-3333-3333-3333-333333333333"
CH = "44444444-4444-4444-4444-444444444444"
TS = "2026-11-05T00:00:00Z"
IN_FLIGHT = "https://maidan.world/problems/idempotency-key-in-flight"


class FakeMaidan:
    """Enough of Maidan for the three claimants: queue-depth, both claim-next
    routes, acknowledge, renew, release and result over REST, and the same
    tools over `/mcp/streamable`. Hands out `threads` claims, then null."""

    def __init__(self, threads: int = 1) -> None:
        self.lock = threading.Lock()
        self.queue = [str(uuid.uuid4()) for _ in range(threads)]
        self.leases: dict[str, str] = {}
        self.claims: list[tuple[str, str]] = []
        self.acks: list[str] = []
        self.renewals: list[str] = []
        self.releases: list[str] = []
        self.results: list[str] = []
        self.transitions: list[str] = []
        self.replays = 0
        self.keys: dict[str, tuple[bytes, int, bytes]] = {}
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def handle_one(self):
                n = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(n) if n else b""
                status, out = fake.answer(self.command, self.path, raw, self.headers.get("Idempotency-Key"))
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(out)))
                self.end_headers()
                self.wfile.write(out)

            do_GET = do_POST = do_PUT = handle_one

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def answer(self, method: str, path: str, raw: bytes, key: str | None) -> tuple[int, bytes]:
        with self.lock:
            fingerprint = method.encode() + b"\n" + path.encode() + b"\n" + raw
            if key is not None and method != "GET":
                if key in self.keys:
                    stored_print, status, out = self.keys[key]
                    if stored_print != fingerprint:
                        return 422, b'{"type":"https://maidan.world/problems/idempotency-key-reused"}'
                    self.replays += 1
                    return status, out
            body = json.loads(raw) if raw else None
            status, value = self.route(method, path, body)
            out = json.dumps(value).encode()
            # As the server: a 5xx or a "not now" 4xx frees the key for a rerun.
            if key is not None and method != "GET" and status < 500 and status not in (408, 409, 425, 429):
                self.keys[key] = (fingerprint, status, out)
            return status, out

    def thread(self, tid: str, lease: str | None) -> dict:
        return {"id": tid, "channel_id": CH, "state": "open", "created_at": TS, "updated_at": TS,
                "assignee_id": "m1", "claim_lease_id": lease,
                "pin": {"uri": "maidan:event/1", "content_hash": "c0ffee"}}

    def claim(self) -> dict | None:
        if not self.queue:
            return None
        tid = self.queue.pop(0)
        lease = str(uuid.uuid4())
        self.leases[tid] = lease
        self.claims.append((tid, lease))
        return self.thread(tid, lease)

    def holder(self, tid: str, body: dict | None, log: list[str]) -> tuple[int, dict]:
        lease = (body or {}).get("claim_lease_id")
        if self.leases.get(tid) != lease:
            return 409, {"type": "https://maidan.world/problems/claim-lost", "status": 409}
        log.append(tid)
        return 200, self.thread(tid, lease)

    def route(self, method: str, path: str, body: object) -> tuple[int, object]:
        parts = path.split("?")[0].strip("/").split("/")
        if method == "GET" and parts[-1] == "queue-depth":
            return 200, {"open": len(self.queue), "ready": len(self.queue), "assigned": 0, "blocked": 0}
        if method == "POST" and parts[-2:] == ["threads", "claim-next"]:
            return 200, self.claim()
        if method == "POST" and parts[0] == "threads" and parts[-2:-1] == ["claim"]:
            tid, verb = parts[1], parts[-1]
            if verb == "acknowledge":
                return self.holder(tid, body, self.acks)
            if verb == "renew":
                return self.holder(tid, body, self.renewals)
            if verb == "release":
                status, out = self.holder(tid, body, self.releases)
                if status == 200:
                    del self.leases[tid]
                return status, out
        if method == "PUT" and parts[0] == "threads" and parts[-1] == "result":
            status, _ = self.holder(parts[1], body, self.results)
            if status != 200:
                return status, {"type": "https://maidan.world/problems/claim-lost", "status": status}
            return 200, {"thread_id": parts[1], "result": body["result"], "produced_by": "m1", "produced_at": TS}
        if method == "POST" and path == "/mcp/streamable":
            return 200, self.mcp(body)
        return 404, {"type": "https://maidan.world/problems/not-found", "status": 404}

    def mcp(self, body: dict) -> dict:
        name = body["params"]["name"]
        args = body["params"].get("arguments", {})
        tid = args.get("thread_id", "")
        if name == "whoami":
            status, value = 200, {"workspace_id": WS, "member_id": "m1"}
        elif name == "claim_next_workspace_thread":
            status, value = 200, self.claim()
        elif name in ("acknowledge_claim", "renew_claim", "release_claim"):
            verb = name.split("_")[0]
            status, value = self.route("POST", f"/threads/{tid}/claim/{verb}", args)
        elif name == "transition_thread":
            self.transitions.append(tid)
            status, value = 200, {"id": tid, "state": "in_review"}
        else:
            status, value = 404, {"detail": f"no tool {name}"}
        text = json.dumps(value)
        return {"jsonrpc": "2.0", "id": body["id"],
                "result": {"content": [{"type": "text", "text": text}], "isError": status >= 400}}


class FaultCase(unittest.TestCase):
    def setUp(self) -> None:
        self.maidan = FakeMaidan()
        self.addCleanup(self.maidan.close)
        self.proxy = faults.FaultProxy(self.maidan.url)
        self.addCleanup(self.proxy.close)

    def plan(self, *rules: dict) -> None:
        self.proxy.set_plan(list(rules))

    def assert_one_claim_released(self) -> None:
        self.assertEqual(len(self.maidan.claims), 1, "claimed exactly once")
        self.assertEqual(self.maidan.leases, {}, "and released")


# ------------------------------------------------------------- seat runner


class SeatRunnerUnderFaults(FaultCase):
    """`run_once` against the proxy. A fake harness and a fake `capped` stand
    in for Codex and `scripts/capped`, as in examples/seat-runner/tests."""

    CLAIM = r"/threads/claim-next$"
    ACK = r"/claim/acknowledge$"
    RELEASE = r"/claim/release$"

    def setUp(self) -> None:
        super().setUp()
        self.tmp = Path(tempfile.mkdtemp())
        self.harness_log = self.tmp / "harness.log"
        self.env = {**os.environ, "MAIDAN_TOKEN": "seat-token", "FAKE_HARNESS_LOG": str(self.harness_log),
                    "FAKE_HARNESS_SECS": "0"}
        self.env.pop("FAKE_HARNESS_IGNORE_TERM", None)
        os.environ.update({k: self.env[k] for k in ("FAKE_HARNESS_LOG", "FAKE_HARNESS_SECS", "MAIDAN_TOKEN")})

    def runner(self, idle_secs: float = 0) -> tuple[seat_runner.Runner, io.StringIO]:
        path = self.tmp / "seats.toml"
        tests = SEAT_DIR / "tests"
        path.write_text(f"""
seats = 1
idle_secs = {idle_secs}
kill_grace_secs = 1
[maidan]
url = "{self.proxy.url}"
workspace_id = "{WS}"
timeout_secs = 1
claim_attempts = 3
claim_backoff_secs = 0.01
release_attempts = 4
release_backoff_secs = 0.01
[harness]
name = "codex"
bin = "{tests / 'fake_harness.py'}"
model = "example-model"
models = ["example-model"]
[caps]
capped = "{tests / 'fake_capped.py'}"
run_secs = 30
[host]
max_load15 = 100000
min_free_gb = 0
[[repos]]
repo = "acme/api"
checkout = "{self.tmp}"
preflight = [["true"]]
""")
        out = io.StringIO()
        return seat_runner.Runner(seat_runner.load_config(path, self.env), out=out), out

    def run_seat(self) -> dict:
        runner, out = self.runner()
        outcome = runner.run_once(0, runner.cfg.repos[0])
        lines = out.getvalue().splitlines()
        self.assertEqual(len(lines), 1, f"one log line per run: {lines}")
        line = json.loads(lines[0])
        self.assertEqual(line["outcome"], outcome)
        return line

    def harness_runs(self) -> int:
        return len(self.harness_log.read_text().splitlines()) if self.harness_log.exists() else 0

    def assert_ran(self, line: dict) -> None:
        self.assertEqual(line["outcome"], "ran", line)
        self.assertEqual(line["release"], 200, line)
        self.assertEqual(self.harness_runs(), 1)
        self.assert_one_claim_released()

    def assert_stopped_without_a_claim(self, line: dict, outcome: str) -> None:
        self.assertEqual(line["outcome"], outcome, line)
        self.assertTrue(line.get("detail"), f"says why: {line}")
        self.assertEqual(self.harness_runs(), 0)
        self.assertEqual(self.maidan.claims, [], "took no claim")

    def test_latency_under_the_timeout_only_slows_the_run(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "latency", "secs": 0.4}})
        self.assert_ran(self.run_seat())

    def test_a_claim_next_that_times_out_is_tried_again(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "timeout", "secs": 2}})
        self.assert_ran(self.run_seat())
        self.assertEqual(len(self.proxy.requests(self.CLAIM)), 2)

    def test_a_dropped_claim_next_is_tried_again(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "drop"}})
        self.assert_ran(self.run_seat())

    def test_a_claim_whose_answer_was_lost_is_answered_again_not_taken_twice(self):
        # The claim lands upstream and its answer never arrives. A retry
        # without the first try's Idempotency-Key would take a second thread
        # and leave the first held for its whole lease.
        self.maidan.queue.append(str(uuid.uuid4()))
        self.plan({"path": self.CLAIM, "fault": {"kind": "drop_after"}})
        line = self.run_seat()
        self.assertEqual(line["outcome"], "ran", line)
        self.assertEqual(len(self.maidan.claims), 1, "the retry was answered from the first claim")
        self.assertEqual(self.maidan.replays, 1)
        self.assertEqual(self.maidan.leases, {})
        keys = {r.headers.get("idempotency-key") for r in self.proxy.requests(self.CLAIM)}
        self.assertEqual(len(keys), 1, "both tries carried one key")
        self.assertNotIn(None, keys)

    def test_a_claim_next_500_502_or_503_is_tried_again(self):
        for status in (500, 502, 503):
            with self.subTest(status=status):
                self.setUp()
                self.plan({"path": self.CLAIM, "fault": {"kind": "status", "status": status}})
                self.assert_ran(self.run_seat())

    def test_a_503_with_retry_after_waits_that_long_before_the_next_try(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "status", "status": 503, "retry_after": "1"}})
        self.assert_ran(self.run_seat())
        first, second = self.proxy.requests(self.CLAIM)
        self.assertGreaterEqual(second.at - first.at, 0.9)

    def test_a_claim_next_that_keeps_failing_stops_after_its_attempts_without_a_claim(self):
        self.plan({"path": self.CLAIM, "times": -1, "fault": {"kind": "status", "status": 503}})
        line = self.run_seat()
        self.assert_stopped_without_a_claim(line, "claim_failed")
        self.assertEqual((line["status"], line["tries"]), (503, 3))
        self.assertEqual(len(self.proxy.requests(self.CLAIM)), 3)

    def test_a_claim_answer_cut_short_is_answered_again(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "truncate", "bytes": 20, "keep_length": True}})
        self.assert_ran(self.run_seat())
        self.assertEqual(self.maidan.replays, 1)

    def test_a_claim_answer_that_is_broken_json_is_answered_again(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "truncate", "bytes": 20}})
        self.assert_ran(self.run_seat())
        self.assertEqual(self.maidan.replays, 1)

    def test_a_json_rpc_error_where_a_claim_belongs_runs_nothing(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "rpc_error", "code": -32603}})
        self.assert_stopped_without_a_claim(self.run_seat(), "claim_failed")

    def test_a_claim_missing_its_lease_or_with_a_mistyped_id_runs_no_harness(self):
        for spec in ({"drop": ["claim_lease_id"]}, {"set": {"id": 42}}, {"set": {"claim_lease_id": None}}):
            with self.subTest(spec=spec):
                self.setUp()
                self.plan({"path": self.CLAIM, "fault": {"kind": "drift", **spec}})
                line = self.run_seat()
                self.assertEqual(line["outcome"], "claim_failed", line)
                self.assertIn("claim_lease_id", line["detail"])
                self.assertEqual(self.harness_runs(), 0)
                self.assertEqual(len(self.maidan.claims), 1, "one claim, not a retry for another")

    def test_a_claim_with_an_extra_field_or_mistyped_attributes_still_runs(self):
        for spec in ({"add": {"new_field": [1, 2]}}, {"set": {"attributes": "review"}}):
            with self.subTest(spec=spec):
                self.setUp()
                self.plan({"path": self.CLAIM, "fault": {"kind": "drift", **spec}})
                self.assert_ran(self.run_seat())

    def test_a_queue_depth_without_a_whole_number_ready_claims_nothing(self):
        # Read as 0 it looked like an idle queue; read with int() a string
        # ended the run in a traceback.
        for spec in ({"drop": ["ready"]}, {"set": {"ready": "lots"}}, {"set": {"ready": True}},
                     {"set": {"ready": -1}}):
            with self.subTest(spec=spec):
                self.setUp()
                self.plan({"path": "/queue-depth$", "fault": {"kind": "drift", **spec}})
                self.assert_stopped_without_a_claim(self.run_seat(), "queue_unreadable")
                self.assertEqual(self.proxy.requests(self.CLAIM), [])

    def test_a_queue_depth_503_is_unreachable_with_its_status(self):
        self.plan({"path": "/queue-depth$", "fault": {"kind": "status", "status": 503}})
        line = self.run_seat()
        self.assert_stopped_without_a_claim(line, "queue_unreachable")
        self.assertIn("503", line["detail"])

    def test_a_lost_acknowledge_is_answered_again(self):
        self.plan({"path": self.ACK, "fault": {"kind": "drop_after"}})
        self.assert_ran(self.run_seat())
        self.assertEqual(len(self.maidan.acks), 1)

    def test_an_acknowledge_that_keeps_failing_releases_and_runs_nothing(self):
        self.plan({"path": self.ACK, "times": -1, "fault": {"kind": "status", "status": 502}})
        line = self.run_seat()
        self.assertEqual((line["outcome"], line["release"]), ("ack_failed", 200), line)
        self.assertEqual(self.harness_runs(), 0)
        self.assert_one_claim_released()

    def test_a_release_answer_cut_short_is_tried_again_and_the_run_keeps_its_outcome(self):
        # An IncompleteRead escaped the release's retries and the run ended
        # in a traceback, logged as `error` instead of `ran`.
        self.plan({"path": self.RELEASE, "fault": {"kind": "truncate", "bytes": 5, "keep_length": True}})
        self.assert_ran(self.run_seat())
        self.assertEqual(len(self.maidan.releases), 1, "released once; the retry was a replay")

    def test_a_release_never_answered_gives_up_after_its_attempts(self):
        self.plan({"path": self.RELEASE, "times": -1, "fault": {"kind": "drop"}})
        line = self.run_seat()
        self.assertEqual((line["outcome"], line["release"]), ("ran", 0), line)
        self.assertEqual(len(self.proxy.requests(self.RELEASE)), 4)

    def test_a_server_failing_everything_does_not_loop_hot(self):
        self.plan({"times": -1, "fault": {"kind": "status", "status": 503}})
        runner, out = self.runner(idle_secs=0.25)
        seat = threading.Thread(target=runner.seat_loop, args=(0, False))
        seat.start()
        time.sleep(1.2)
        runner.stop.set()
        seat.join(5)
        self.assertFalse(seat.is_alive())
        lines = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertTrue(2 <= len(lines) <= 7, f"one run per idle period: {len(lines)}")
        self.assertEqual({line["outcome"] for line in lines}, {"queue_unreachable"})
        self.assertEqual(len(self.proxy.requests()), len(lines), "one request per run")


# ----------------------------------------------------------- Python SDK


class SdkClaimLoopUnderFaults(FaultCase):
    """The claim loop an SDK user writes: claim, renew, set the result."""

    CLAIM = r"/threads/claim-next$"
    RESULT = r"/result$"

    def client(self, **kw) -> maidan.Client:
        c = maidan.Client(self.proxy.url, "t", timeout=1, max_retries=2, **kw)
        self.sleeps: list[float] = []
        c._sleep = self.sleeps.append
        return c

    def work(self, c: maidan.Client):
        claim = c.claim_next_thread(CH, {"lease_secs": 60})
        if claim is None:
            return None
        c.renew_claim(claim.id, claim.claim_lease_id, 60)
        c.threads.set_result(claim.id, {"ok": True}, claim.claim_lease_id)
        return claim

    def assert_worked_once(self, claim) -> None:
        self.assertIsNotNone(claim)
        self.assertEqual(len(self.maidan.claims), 1)
        self.assertEqual(self.maidan.results, [claim.id], "one result, written once")

    def test_latency_under_the_timeout_only_slows_the_loop(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "latency", "secs": 0.4}})
        self.assert_worked_once(self.work(self.client()))

    def test_a_claim_next_that_times_out_is_tried_again(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "timeout", "secs": 2}})
        self.assert_worked_once(self.work(self.client()))

    def test_a_claim_whose_answer_was_lost_is_answered_again_not_taken_twice(self):
        self.maidan.queue.append(str(uuid.uuid4()))
        self.plan({"path": self.CLAIM, "fault": {"kind": "drop_after"}})
        self.assert_worked_once(self.work(self.client()))
        self.assertEqual(self.maidan.replays, 1)

    def test_a_result_whose_answer_was_lost_is_written_once(self):
        self.plan({"path": self.RESULT, "fault": {"kind": "drop_after"}})
        self.assert_worked_once(self.work(self.client()))
        self.assertEqual(self.maidan.replays, 1)

    def test_a_500_502_or_503_is_tried_again(self):
        for status in (500, 502, 503):
            with self.subTest(status=status):
                self.setUp()
                self.plan({"path": self.CLAIM, "fault": {"kind": "status", "status": status}})
                self.assert_worked_once(self.work(self.client()))

    def test_retry_after_on_a_503_sets_the_wait(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "status", "status": 503, "retry_after": "2"}})
        self.assert_worked_once(self.work(self.client()))
        self.assertEqual(self.sleeps, [2.0])

    def test_a_claim_next_that_keeps_failing_raises_once_without_a_claim(self):
        self.plan({"path": self.CLAIM, "times": -1, "fault": {"kind": "status", "status": 503}})
        with self.assertRaises(maidan.MaidanError) as e:
            self.work(self.client())
        self.assertEqual(e.exception.status, 503)
        self.assertEqual(len(self.proxy.requests(self.CLAIM)), 3, "max_retries bounds it")
        self.assertEqual(self.maidan.claims, [])

    def test_no_answer_after_the_last_try_is_a_maidan_error_with_status_0(self):
        # It was the bare URLError, which `except MaidanError` missed, though
        # MaidanError documents status 0 as exactly this.
        self.plan({"path": self.CLAIM, "times": -1, "fault": {"kind": "drop"}})
        with self.assertRaises(maidan.MaidanError) as e:
            self.work(self.client())
        self.assertEqual(e.exception.status, 0)
        self.assertIn("no answer after 3 tries", str(e.exception))
        self.assertEqual(self.maidan.claims, [])

    def test_a_claim_answer_cut_short_is_answered_again(self):
        # http.client.IncompleteRead is no OSError: it escaped unretried, and
        # a caller that retried by hand claimed a second thread.
        self.maidan.queue.append(str(uuid.uuid4()))
        self.plan({"path": self.CLAIM, "fault": {"kind": "truncate", "bytes": 20, "keep_length": True}})
        self.assert_worked_once(self.work(self.client()))

    def test_a_claim_answer_that_is_broken_json_is_answered_again(self):
        self.maidan.queue.append(str(uuid.uuid4()))
        self.plan({"path": self.CLAIM, "fault": {"kind": "truncate", "bytes": 20}})
        self.assert_worked_once(self.work(self.client()))

    def test_a_claim_answer_that_stays_broken_raises_unreadable_after_one_claim(self):
        self.maidan.queue.append(str(uuid.uuid4()))
        self.plan({"path": self.CLAIM, "times": -1, "fault": {"kind": "truncate", "bytes": 20}})
        with self.assertRaises(maidan.UnreadableResponseError) as e:
            self.work(self.client())
        self.assertIsInstance(e.exception, maidan.MaidanError)
        self.assertEqual(len(self.maidan.claims), 1, "three tries, one key, one claim")

    def test_a_drifted_claim_raises_a_maidan_error_and_claims_nothing_more(self):
        for spec in ({"drop": ["pin"]}, {"drop": ["claim_lease_id"]}, {"set": {"id": 42}},
                     {"set": {"pin": "not-a-pin"}}):
            with self.subTest(spec=spec):
                self.setUp()
                self.plan({"path": self.CLAIM, "fault": {"kind": "drift", **spec}})
                with self.assertRaises(maidan.UnreadableResponseError) as e:
                    self.work(self.client())
                self.assertIn("claim", str(e.exception))
                self.assertEqual(len(self.maidan.claims), 1)
                self.assertEqual(self.maidan.results, [])

    def test_a_json_rpc_error_where_a_claim_belongs_raises_a_maidan_error(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "rpc_error"}})
        with self.assertRaises(maidan.UnreadableResponseError):
            self.work(self.client())
        self.assertEqual(self.maidan.claims, [])

    def test_an_extra_field_is_kept_and_the_loop_goes_on(self):
        self.plan({"path": self.CLAIM, "fault": {"kind": "drift", "add": {"new_field": 1}}})
        claim = self.work(self.client())
        self.assert_worked_once(claim)
        self.assertEqual(claim.extra, {"new_field": 1})

    def test_a_drifted_renewal_raises_a_maidan_error_before_any_result(self):
        self.plan({"path": r"/claim/renew$", "fault": {"kind": "drift", "drop": ["channel_id"]}})
        with self.assertRaises(maidan.UnreadableResponseError):
            self.work(self.client())
        self.assertEqual(self.maidan.results, [])


# --------------------------------------------- the OpenRouter MCP claimant


class McpClaimantUnderFaults(FaultCase):
    """`examples/openrouter/claimant.py` claims, acknowledges, renews and
    releases over `/mcp/streamable`. Its model work (`deliver`) is stubbed:
    these tests are about the claim around it."""

    def setUp(self) -> None:
        super().setUp()
        with self.maidan.lock:
            self.maidan.queue.append(str(uuid.uuid4()))
        self.second = self.maidan.queue[-1]
        self.tmp = tempfile.mkdtemp()
        self.delivered: list[str] = []
        saved = (claimant.deliver, claimant.report_usage)

        def restore():
            claimant.deliver, claimant.report_usage = saved

        self.addCleanup(restore)
        claimant.deliver = lambda args, m, key, tid, *rest: self.delivered.append(tid)
        claimant.report_usage = lambda *a: None

    def tool(self, name: str) -> dict:
        return {"path": "^/mcp/streamable$", "tool": name}

    def run_main(self) -> tuple[int, list[str]]:
        env = {"MAIDAN_URL": self.proxy.url, "MAIDAN_TOKEN": "t", "OPENROUTER_API_KEY": "k"}
        err, out = io.StringIO(), io.StringIO()
        saved = {k: os.environ.get(k) for k in env}
        os.environ.update(env)
        try:
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
                code = claimant.main(["--model", "x-ai/grok-4", "--repo", "acme/api",
                                      "--repo-dir", self.tmp, "--wait-ms", "0"])
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
        return code, err.getvalue().splitlines()

    def assert_one_line(self, code: int, lines: list[str], prefix: str = "claimant: ") -> None:
        self.assertEqual(code, 1)
        self.assertEqual(len(lines), 1, f"one line, no traceback: {lines}")
        self.assertTrue(lines[0].startswith(prefix), lines)

    def test_the_claim_round_trip_without_faults(self):
        code, lines = self.run_main()
        self.assertEqual((code, lines), (0, []))
        self.assertEqual(len(self.delivered), 1)
        self.assert_one_claim_released()

    def test_latency_only_slows_the_claim(self):
        self.plan({**self.tool("claim_next_workspace_thread"), "fault": {"kind": "latency", "secs": 0.4}})
        self.assertEqual(self.run_main(), (0, []))
        self.assert_one_claim_released()

    def test_a_json_rpc_error_on_the_claim_stops_with_one_line_and_no_claim(self):
        self.plan({**self.tool("claim_next_workspace_thread"),
                   "fault": {"kind": "rpc_error", "code": -32603, "message": "upstream unavailable"}})
        code, lines = self.run_main()
        self.assert_one_line(code, lines)
        self.assertIn("upstream unavailable", lines[0])
        self.assertEqual(self.maidan.claims, [])

    def _claim_keys(self) -> set[str]:
        return {seen.headers.get("idempotency-key")
                for seen in self.proxy.seen
                if seen.tool == "claim_next_workspace_thread" and seen.headers.get("idempotency-key")}

    def assert_recovered_one(self) -> None:
        code, lines = self.run_main()
        self.assertEqual((code, lines), (0, []), lines)
        self.assertEqual(len(self.delivered), 1)
        self.assertEqual(len(self.maidan.claims), 1, "one claim, never a second thread")
        self.assertEqual(self.maidan.leases, {})
        self.assertIn(self.second, self.maidan.queue)
        self.assertEqual(len(self._claim_keys()), 1)

    def test_a_lost_or_cut_short_claim_is_retried_once(self):
        for fault in ({"kind": "drop"},
                      {"kind": "drop_after"},
                      {"kind": "status", "status": 503, "retry_after": "0"},
                      {"kind": "status", "status": 409, "retry_after": "0",
                       "type": "https://maidan.world/problems/idempotency-key-in-flight"},
                      {"kind": "truncate", "bytes": 30, "keep_length": True},
                      {"kind": "truncate", "bytes": 30, "keep_length": False}):
            with self.subTest(fault=fault):
                self.setUp()
                self.plan({**self.tool("claim_next_workspace_thread"), "fault": fault})
                self.assert_recovered_one()

    def test_an_exhausted_claim_stops_with_one_line(self):
        self.plan({**self.tool("claim_next_workspace_thread"), "times": 3, "fault": {"kind": "drop"}})
        code, lines = self.run_main()
        self.assert_one_line(code, lines)
        self.assertIn("no answer", lines[0])
        self.assertNotIn("Traceback", "\n".join(lines))
        self.assertEqual(self.delivered, [])
        self.assertEqual(self.maidan.claims, [])

    def test_a_claim_missing_its_lease_or_id_stops_with_one_line(self):
        # It was a KeyError traceback.
        for spec in ({"drop": ["claim_lease_id"]}, {"set": {"id": 42}}):
            with self.subTest(spec=spec):
                self.setUp()
                self.plan({**self.tool("claim_next_workspace_thread"), "fault": {"kind": "drift", **spec}})
                code, lines = self.run_main()
                self.assert_one_line(code, lines)
                self.assertIn("claim_lease_id", lines[0])
                self.assertEqual(self.delivered, [])

    def test_a_whoami_that_drifted_stops_before_any_claim(self):
        self.plan({**self.tool("whoami"), "fault": {"kind": "drift", "drop": ["workspace_id"]}})
        code, lines = self.run_main()
        self.assert_one_line(code, lines)
        self.assertEqual(self.maidan.claims, [])

    def test_a_json_rpc_error_on_acknowledge_releases_the_claim(self):
        self.plan({**self.tool("acknowledge_claim"), "fault": {"kind": "rpc_error"}})
        code, lines = self.run_main()
        self.assert_one_line(code, lines)
        self.assertEqual(self.delivered, [])
        self.assert_one_claim_released()

    def test_a_body_that_arrives_in_two_bursts_is_read_whole(self):
        # A non-blocking drain used to treat "nothing ready yet" as end of file
        # and return only the first burst.
        body = b'{"ok":true,"pad":"' + (b"x" * 400) + b'"}'

        class Burst(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body[:12])
                self.wfile.flush()
                time.sleep(0.05)
                self.wfile.write(body[12:])
                self.wfile.flush()

            def log_message(self, _fmt, *_args):
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Burst)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}/"
        with urllib.request.urlopen(url, timeout=2) as resp:
            got = claimant.read_until(resp, 2)
        self.assertEqual(got, body)

    def test_a_drip_past_the_deadline_stops(self):
        self.plan({**self.tool("whoami"),
                   "fault": {"kind": "drip", "chunk_bytes": 4, "pause_ms": 80}})
        api = claimant.Maidan(self.proxy.url, "t", max_retries=0, sleep=lambda _s: None)
        started = time.monotonic()
        with self.assertRaises(claimant.MaidanError):
            api.tool("whoami", {}, timeout=0.15)
        elapsed = time.monotonic() - started
        self.assertLess(elapsed, 0.6, f"the deadline should cut a drip short, took {elapsed:.2f}s")
        self.assertEqual(self.maidan.claims, [])

    def test_a_claim_lost_conflict_is_not_retried(self):
        self.plan({**self.tool("acknowledge_claim"), "times": 1, "fault": {
            "kind": "status", "status": 409,
            "type": "https://maidan.world/problems/claim-lost",
        }})
        code, lines = self.run_main()
        seen = [(s.tool, s.fault, s.status) for s in self.proxy.seen]
        self.assertEqual(code, 1, (lines, seen))
        self.assertEqual(len(lines), 1, lines)
        acks = [s for s in self.proxy.seen if s.tool == "acknowledge_claim"]
        self.assertEqual(len(acks), 1, seen)

    def test_a_release_that_gets_no_answer_says_so_in_one_line(self):
        # Every attempt is dropped, so the retries run out and the release says so.
        self.plan({**self.tool("release_claim"), "times": -1, "fault": {"kind": "drop"}})
        code, lines = self.run_main()
        self.assertEqual(code, 0)
        self.assertEqual(len(lines), 1, lines)
        self.assertTrue(lines[0].startswith("release: "), lines)

    def test_a_renewal_that_gets_no_answer_marks_the_claim_lost(self):
        # The renewer caught only Fatal, so a dropped connection ended its
        # thread with `lost` unset and the claimant delivered on a dead lease.
        self.plan({**self.tool("renew_claim"), "times": -1, "fault": {"kind": "drop"}})
        api = claimant.Maidan(self.proxy.url, "t")
        claim = api.tool("claim_next_workspace_thread", {"workspace_id": WS})
        renewer = claimant.Renewer(api, claim["id"], claim["claim_lease_id"], 0.3)
        renewer.start()
        renewer.join(5)
        self.assertFalse(renewer.is_alive())
        self.assertIsNotNone(renewer.lost)
        self.assertIn("no answer", renewer.lost)


# ------------------------------------------------------------ the proxy


class FaultProxyItself(FaultCase):
    def test_rules_match_by_request_number_skip_and_times(self):
        self.plan({"n": 2, "fault": {"kind": "status", "status": 500}},
                  {"path": "/queue-depth$", "skip": 2, "times": 1, "fault": {"kind": "status", "status": 502}})
        api = seat_runner.Maidan(self.proxy.url, "t", timeout=2)
        statuses = [api.call("GET", f"/workspaces/{WS}/queue-depth")[0] for _ in range(5)]
        # Request 2 goes to the first rule, so the second sees 1, 3 and 4.
        self.assertEqual(statuses, [200, 500, 200, 502, 200])

    def test_a_drifted_tool_result_changes_inside_the_text_content(self):
        self.plan({"tool": "whoami", "fault": {"kind": "drift", "drop": ["workspace_id"], "add": {"x": 1}}})
        api = claimant.Maidan(self.proxy.url, "t")
        self.assertEqual(api.tool("whoami", {}), {"member_id": "m1", "x": 1})

    def test_a_rule_without_a_known_fault_is_refused(self):
        with self.assertRaises(ValueError):
            faults.Rule.from_dict({"fault": {"kind": "gremlins"}})


def _bin(name: str, env_name: str) -> str | None:
    chosen = os.environ.get(env_name)
    if chosen:
        return chosen
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    path = target / "debug" / name
    return str(path) if path.is_file() else None


class RealServerMcpFaults(unittest.TestCase):
    """The OpenRouter claimant against maidan-server, with the fault proxy in front.

    Skipped only when the binaries are not on disk. This checkout builds them
    under target/debug, so the case runs here.
    """

    @classmethod
    def setUpClass(cls) -> None:
        cls.server_bin = _bin("maidan-server", "MAIDAN_SERVER_BIN")
        cls.cli_bin = _bin("maidan", "MAIDAN_BIN")
        if not cls.server_bin or not cls.cli_bin:
            raise unittest.SkipTest("maidan-server and maidan are not built")

    def test_a_lost_claim_against_the_real_server_takes_one_thread(self) -> None:
        import socket as socket_mod
        import urllib.request

        work = Path(tempfile.mkdtemp())
        db = work / "maidan.db"
        kek = "11" * 32
        env = {**os.environ, "DATABASE_URL": f"sqlite:///{db}?mode=rwc", "MAIDAN_CONTENT_KEK": kek,
               "MAIDAN_SESSION_SECRET": "fault-test-session-secret-32bytes!",
               "MAIDAN_BOOTSTRAP": "1", "MAIDAN_LOG": "error"}
        env.pop("MAIDAN_ENV", None)
        init = subprocess.run(
            [self.cli_bin, "init", "--database-url", env["DATABASE_URL"],
             "--workspace", "faults", "--admin-handle", "admin"],
            env=env, capture_output=True, text=True, timeout=60)
        self.assertEqual(init.returncode, 0, init.stderr[-500:])
        token = next(line.strip() for line in reversed(init.stdout.splitlines()) if line.strip().startswith("maid_"))
        sock = socket_mod.socket()
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
        sock.close()
        server_env = {**env, "MAIDAN_BIND": f"127.0.0.1:{port}",
                      "ARTIFACT_LOCALFS_ROOT": str(work / "artifacts")}
        proc = subprocess.Popen(
            [self.server_bin], env=server_env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)

        def stop_server() -> None:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)

        self.addCleanup(stop_server)
        base = f"http://127.0.0.1:{port}"
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                urllib.request.urlopen(base + "/health/ready", timeout=0.5).close()
                break
            except OSError:
                if proc.poll() is not None:
                    self.fail(proc.stderr.read()[-800:] if proc.stderr else "server exited")
                time.sleep(0.2)
        else:
            self.fail("server did not become ready")

        def call(method, path, body=None, bearer=token):
            data = json.dumps(body).encode() if body is not None else None
            req = urllib.request.Request(base + path, data=data, method=method)
            req.add_header("Authorization", f"Bearer {bearer}")
            if data is not None:
                req.add_header("Content-Type", "application/json")
            with urllib.request.urlopen(req, timeout=10) as resp:
                raw = resp.read()
                return json.loads(raw) if raw else None

        me = call("GET", "/me")
        workspace = me["workspace_id"]
        agent = call("POST", f"/workspaces/{workspace}/members", {"handle": "seat", "kind": "agent"})
        minted = call("POST", f"/workspaces/{workspace}/members/{agent['id']}/tokens",
                      {"label": "seat", "capabilities": me["known_capabilities"]})
        seat = minted["secret"]
        channel = call("POST", f"/workspaces/{workspace}/channels", {"name": "work"}, bearer=seat)
        call("POST", f"/channels/{channel['id']}/members", {"member_id": agent["id"]}, bearer=token)
        made = {"n": 0}

        def two_threads():
            made["n"] += 1
            repo = f"acme/run-{made['n']}"
            out = []
            for title in ("one", "two"):
                out.append(call("POST", f"/channels/{channel['id']}/threads", {
                    "title": f"{title}-{made['n']}",
                    "attributes": {"kind": "change", "engine": "session"},
                    "footprint": {"repo": repo, "mode": "exclusive",
                                  "items": [{"kind": "branch", "value": f"feature/{title}-{made['n']}"}]},
                }, bearer=seat))
            return repo, out

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def _ok(self, doc):
                raw = json.dumps(doc).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self):
                self._ok({"data": []})

            def do_POST(self):
                self._ok({"choices": [{"message": {"role": "assistant", "content": "done"}}], "usage": {}})

        router = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        router.daemon_threads = True
        threading.Thread(target=router.serve_forever, daemon=True).start()
        self.addCleanup(router.shutdown)

        saved = (claimant.deliver, claimant.report_usage)

        def deliver(args, m, key, tid, repo, checkout, usage, renewer):
            m.tool("set_thread_result", {
                "thread_id": tid, "claim_lease_id": renewer.lease_id,
                "result": {"schema": "maidan.waiter.result/1", "result_kind": "pi.change.result/1",
                           "status": "no_change", "error_detail": "fault test", "deliver_to": []},
            })

        claimant.deliver = deliver
        claimant.report_usage = lambda *a: None
        self.addCleanup(lambda: setattr(claimant, "deliver", saved[0]))
        self.addCleanup(lambda: setattr(claimant, "report_usage", saved[1]))

        for fault in ({"kind": "drop_after"},
                      {"kind": "truncate", "bytes": 40, "keep_length": False},
                      {"kind": "truncate", "bytes": 40, "keep_length": True},
                      {"kind": "status", "status": 503, "retry_after": "0"}):
            with self.subTest(fault=fault["kind"] + str(fault.get("keep_length", ""))):
                repo, threads = two_threads()
                proxy = faults.FaultProxy(base)
                self.addCleanup(proxy.close)
                proxy.set_plan([{"path": "^/mcp/streamable$", "tool": "claim_next_workspace_thread",
                                 "fault": fault}])
                err = io.StringIO()
                env_saved = {k: os.environ.get(k) for k in ("MAIDAN_URL", "MAIDAN_TOKEN", "OPENROUTER_API_KEY")}
                os.environ["MAIDAN_URL"] = proxy.url
                os.environ["MAIDAN_TOKEN"] = seat
                os.environ["OPENROUTER_API_KEY"] = "test-key"
                try:
                    with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                        code = claimant.main([
                            "--model", "x-ai/grok-4", "--repo", repo, "--repo-dir", str(work),
                            "--wait-ms", "0", "--lease-secs", "60",
                            "--openrouter-url", f"http://127.0.0.1:{router.server_address[1]}/api/v1",
                        ])
                finally:
                    for key, value in env_saved.items():
                        if value is None:
                            os.environ.pop(key, None)
                        else:
                            os.environ[key] = value
                self.assertEqual(code, 0, err.getvalue())
                listed = call("GET", f"/channels/{channel['id']}/threads", bearer=seat)
                rows = listed if isinstance(listed, list) else listed.get("threads", listed)
                by_id = {row["id"]: row for row in rows}
                pair = [by_id[thread["id"]] for thread in threads]
                states = sorted(row["state"] for row in pair)
                self.assertEqual(states, ["in_review", "open"], pair)
                reviewed = next(row for row in pair if row["state"] == "in_review")
                waiting = next(row for row in pair if row["state"] == "open")
                self.assertIsNone(reviewed.get("claim_lease_id"))
                self.assertIsNone(waiting.get("claim_lease_id"))
                result = call("GET", f"/threads/{reviewed['id']}/result", bearer=seat)
                self.assertEqual(result["result"]["status"], "no_change")


if __name__ == "__main__":
    unittest.main()
