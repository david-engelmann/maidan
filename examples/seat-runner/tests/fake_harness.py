#!/usr/bin/env python3
"""Stand-in for `codex`: records its argv, pid and whether it got the seat
token, then sleeps FAKE_HARNESS_SECS. FAKE_HARNESS_IGNORE_TERM=1 makes it
ignore SIGTERM, like a harness that will not stop."""
import json
import os
import signal
import sys
import time

if os.environ.get("FAKE_HARNESS_IGNORE_TERM") == "1":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
with open(os.environ["FAKE_HARNESS_LOG"], "a") as f:
    f.write(json.dumps({"argv": sys.argv[1:], "pid": os.getpid(),
                        "token": os.environ.get("MAIDAN_TOKEN")}) + "\n")
time.sleep(float(os.environ.get("FAKE_HARNESS_SECS", "0")))
