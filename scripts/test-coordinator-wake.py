#!/usr/bin/env python3
"""Tests for coordinator-wake.sh. Run: python3 scripts/test-coordinator-wake.py

The script runs against a fake /mcp/coordinator that answers the three tools
it calls (whoami, list_notifications, wait_for_notification) from a script of
answers, and records every request. Standard library only; needs bash, curl
and jq, as the script does."""

import json
import os
import subprocess
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

SCRIPT = Path(__file__).with_name("coordinator-wake.sh")
TOKEN = "maidan-test-token-do-not-print"
MEMBER = "0192f000-0000-7000-8000-00000000c00d"
THREAD = "0192f000-0000-7000-8000-0000000071ea"


def note(log_id, kind, reason="", title="fix the flaky test", read=False):
    return {
        "id": f"0192f000-0000-7000-8000-{log_id:012d}",
        "workspace_id": "0192f000-0000-7000-8000-00000000000a",
        "member_id": MEMBER,
        "kind": kind,
        "source_log_id": log_id,
        "channel_id": None,
        "thread_id": THREAD,
        "message_id": None,
        "actor_id": None,
        "created_at": "2026-11-10T12:00:00.123456Z",
        "read_at": "2026-11-10T12:01:00Z" if read else None,
        "title": title,
        "reason": reason,
    }


class Fake:
    """What the fake server answers, and what it was asked."""

    def __init__(self, notes=(), waits=(), also=None):
        self.notes = list(notes)
        self.waits = list(waits)
        # Rows the router writes alongside the one a wait returns, by its
        # source_log_id: what piles up while the coordinator sleeps.
        self.also = also or {}
        self.calls = []
        self.lock = threading.Lock()

    def answer(self, name, args):
        if name == "whoami":
            return {"member_id": MEMBER}
        if name == "list_notifications":
            rows = sorted(self.notes, key=lambda n: n["source_log_id"], reverse=True)
            if args.get("unread_only"):
                rows = [n for n in rows if n["read_at"] is None]
            return rows[: args.get("limit", 50)]
        if name == "wait_for_notification":
            if not self.waits:
                return None
            nxt = self.waits.pop(0)
            if isinstance(nxt, dict):
                # The router wrote it: it is in the ledger from now on.
                self.notes.append(nxt)
                self.notes.extend(self.also.get(nxt["source_log_id"], []))
            return nxt
        raise AssertionError(f"unexpected tool {name}")


def serve(fake):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            raw = self.rfile.read(int(self.headers["Content-Length"]))
            body = json.loads(raw)
            with fake.lock:
                fake.calls.append(
                    {
                        "path": self.path,
                        "auth": self.headers.get("Authorization"),
                        "body": body,
                    }
                )
                if self.headers.get("Authorization") != f"Bearer {TOKEN}":
                    return self.reply(401, {"error": "unauthorized"})
                params = body["params"]
                nxt = None
                if params["name"] == "wait_for_notification" and fake.waits:
                    nxt = fake.waits[0]
                if isinstance(nxt, int):
                    fake.waits.pop(0)
                    return self.reply(nxt, {"error": "unavailable"})
                result = fake.answer(params["name"], params["arguments"])
            if params["name"] == "wait_for_notification" and result is None:
                # A real wait holds the line; a short one keeps the loop honest.
                time.sleep(min(params["arguments"]["timeout_ms"] / 1000, 0.2))
            self.reply(
                200,
                {
                    "jsonrpc": "2.0",
                    "id": body["id"],
                    "result": {
                        "content": [{"type": "text", "text": json.dumps(result)}],
                        "isError": False,
                    },
                },
            )

        def reply(self, status, payload):
            data = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def stop(server):
    server.shutdown()
    server.server_close()


def run(args, url="http://127.0.0.1:9", token=TOKEN, timeout=60):
    env = {k: v for k, v in os.environ.items() if not k.startswith("MAIDAN_")}
    env.update({"MAIDAN_URL": url, "MAIDAN_TOKEN": token})
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        timeout=timeout,
    )


class CoordinatorWake(unittest.TestCase):
    def against(self, fake, args=()):
        server = serve(fake)
        self.addCleanup(stop, server)
        url = f"http://127.0.0.1:{server.server_address[1]}"
        out = run(list(args), url=url)
        self.assertNotIn(TOKEN, out.stdout + out.stderr, "the token was printed")
        return out

    def tools(self, fake):
        return [c["body"]["params"]["name"] for c in fake.calls]

    def test_dry_run_prints_the_request_bodies_without_the_token(self):
        out = run(["--dry-run"], url="https://maidan.example")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertNotIn(TOKEN, out.stdout + out.stderr)
        self.assertIn("POST https://maidan.example/mcp/coordinator", out.stdout)
        bodies = [json.loads(line) for line in out.stdout.splitlines() if line.startswith("{")]
        names = [b["params"]["name"] for b in bodies]
        self.assertEqual(
            names, ["whoami", "list_notifications", "list_notifications", "wait_for_notification"]
        )
        for b in bodies:
            self.assertEqual((b["jsonrpc"], b["method"]), ("2.0", "tools/call"))
        wait = bodies[-1]["params"]["arguments"]
        self.assertEqual(wait["timeout_ms"], 300000, "each wait uses the longest timeout")
        self.assertIn("since_log_id", wait)
        self.assertTrue(bodies[1]["params"]["arguments"]["unread_only"])

    def test_dry_run_with_a_member_skips_whoami(self):
        out = run(["--dry-run", "--member", MEMBER])
        names = [
            json.loads(line)["params"]["name"]
            for line in out.stdout.splitlines()
            if line.startswith("{")
        ]
        self.assertNotIn("whoami", names)
        self.assertIn(MEMBER, out.stdout)

    def test_an_attention_notification_wakes_it_with_a_digest_of_everything_pending(self):
        # An older failed check is written while it waits, beside the question.
        older = note(3, "thread_notice", "CI failed on acme/app#7: unit tests")
        fake = Fake(
            notes=[note(5, "message_posted", read=True)],
            waits=[
                note(6, "message_posted"),
                None,
                note(7, "status_declared", "needs_input: which branch?"),
            ],
            also={7: [older]},
        )
        out = self.against(fake)
        self.assertEqual(out.returncode, 0, out.stderr)
        lines = out.stdout.strip().splitlines()
        self.assertEqual(len(lines), 3, out.stdout)
        self.assertIn("thread_notice", lines[0], "oldest first")
        self.assertIn(f"thread={THREAD}", lines[1])
        self.assertIn('"fix the flaky test"', lines[1])
        self.assertIn("needs_input: which branch?", lines[1])
        self.assertIn("ago", lines[1])
        self.assertNotIn("message_posted", out.stdout, "other kinds are not in the digest")
        self.assertEqual(lines[2], "2 pending: 1 status_declared, 1 thread_notice")
        self.assertNotIn("mark_notification_read", self.tools(fake), "it never marks read")
        waits = [
            c["body"]["params"]["arguments"]
            for c in fake.calls
            if c["body"]["params"]["name"] == "wait_for_notification"
        ]
        self.assertEqual([w["since_log_id"] for w in waits], [5, 6, 6])
        self.assertTrue(all(w["timeout_ms"] == 300000 for w in waits))
        self.assertTrue(all(c["path"] == "/mcp/coordinator" for c in fake.calls))

    def test_a_late_row_below_the_cursor_still_wakes_it(self):
        # The router wrote a stall late, with a source_log_id below the row
        # the wait returned, so no later wait would ever return it.
        fake = Fake(
            notes=[note(5, "message_posted", read=True)],
            waits=[note(6, "message_posted")],
            also={6: [note(4, "claim_stalled", "no progress for 20m")]},
        )
        out = self.against(fake, ["--max-wait", "5"])
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("1 pending: 1 claim_stalled", out.stdout)

    def test_an_unread_attention_notification_wakes_it_without_waiting(self):
        fake = Fake(notes=[note(9, "thread_blocked", "blocked (human)")])
        out = self.against(fake)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("1 pending: 1 thread_blocked", out.stdout)
        self.assertNotIn("wait_for_notification", self.tools(fake))

    def test_kinds_outside_the_attention_set_keep_it_waiting_until_max_wait(self):
        fake = Fake(waits=[note(1, "message_posted"), note(2, "thread_landed")])
        out = self.against(fake, ["--max-wait", "2"])
        self.assertEqual(out.returncode, 2, out.stderr)
        self.assertEqual(out.stdout.strip(), "nothing pending")

    def test_kinds_narrows_what_wakes_it(self):
        fake = Fake(waits=[note(1, "mention_recorded"), note(2, "approval_requested")])
        out = self.against(fake, ["--kinds", "approval_requested"])
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("1 pending: 1 approval_requested", out.stdout)
        self.assertNotIn("mention_recorded", out.stdout)

    def test_a_refused_token_exits_1_and_says_so(self):
        server = serve(Fake())
        self.addCleanup(stop, server)
        out = run([], url=f"http://127.0.0.1:{server.server_address[1]}", token="wrong")
        self.assertEqual(out.returncode, 1)
        self.assertIn("token was refused (HTTP 401)", out.stderr)
        self.assertNotIn("wrong", out.stdout + out.stderr, "the token was printed")

    def test_a_transient_error_is_retried(self):
        fake = Fake(waits=[503, note(4, "claim_stalled", "claim stalled (no_progress)")])
        out = self.against(fake)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("HTTP 503; retrying", out.stderr)
        self.assertIn("1 pending: 1 claim_stalled", out.stdout)

    def test_an_unreachable_server_gives_up_at_max_wait(self):
        out = run(["--max-wait", "1", "--member", MEMBER], url="http://127.0.0.1:9")
        self.assertEqual(out.returncode, 2, out.stderr)
        self.assertEqual(out.stdout.strip(), "nothing pending")
        self.assertNotIn(TOKEN, out.stdout + out.stderr)


if __name__ == "__main__":
    unittest.main()
