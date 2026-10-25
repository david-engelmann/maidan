#!/usr/bin/env python3
"""Stand-in for `codex`: records its argv, then sleeps FAKE_HARNESS_SECS."""
import json
import os
import sys
import time

with open(os.environ["FAKE_HARNESS_LOG"], "a") as f:
    f.write(json.dumps({"argv": sys.argv[1:], "pid": os.getpid()}) + "\n")
time.sleep(float(os.environ.get("FAKE_HARNESS_SECS", "0")))
