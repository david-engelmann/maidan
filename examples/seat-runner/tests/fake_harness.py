#!/usr/bin/env python3
"""Stand-in for `codex`, `claude`, `gemini` and `zcode`: records its argv,
pid, whether it got the seat token, the variables a seat's `env` can set, and
the files a gemini or zcode seat is handed (path, modes and text), then sleeps
FAKE_HARNESS_SECS and exits FAKE_HARNESS_EXIT. FAKE_HARNESS_IGNORE_TERM=1
makes it ignore SIGTERM, like a harness that will not stop."""
import json
import os
import signal
import stat
import sys
import time

if os.environ.get("FAKE_HARNESS_IGNORE_TERM") == "1":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)


def describe(path):
    """A handed file: its text, its mode, and the modes of the directories
    from it up to the run's private directory."""
    dirs, d = [], os.path.dirname(path)
    while os.path.basename(d) and not os.path.basename(d).startswith("run-"):
        dirs.append(stat.S_IMODE(os.stat(d).st_mode))
        d = os.path.dirname(d)
    dirs.append(stat.S_IMODE(os.stat(d).st_mode))
    return {"path": path, "mode": stat.S_IMODE(os.stat(path).st_mode),
            "dir_mode": dirs[0], "dir_modes": dirs, "text": open(path).read()}


settings = None
path = os.environ.get("GEMINI_CLI_SYSTEM_SETTINGS_PATH")
if path:
    settings = describe(path)
zcode = None
if os.environ.get("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE"):
    zcode = {"provider": describe(os.environ["ZCODE_PERSONAL_PROVIDER_CONFIG_FILE"]),
             "runtime": describe(os.path.join(os.environ["HOME"], ".zcode", "cli", "config.json"))}
seen = ("ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ZAI_API_KEY",
        "GEMINI_CLI_TRUST_WORKSPACE", "HOME", "ZCODE_DATA_BASE_DIR", "CARGO_HOME", "RUSTUP_HOME",
        "GIT_CONFIG_GLOBAL", "XDG_CONFIG_HOME")
with open(os.environ["FAKE_HARNESS_LOG"], "a") as f:
    f.write(json.dumps({"argv": sys.argv[1:], "pid": os.getpid(),
                        "token": os.environ.get("MAIDAN_TOKEN"),
                        "env": {k: os.environ[k] for k in seen if k in os.environ},
                        "settings": settings, "zcode": zcode}) + "\n")
time.sleep(float(os.environ.get("FAKE_HARNESS_SECS", "0")))
sys.exit(int(os.environ.get("FAKE_HARNESS_EXIT", "0")))
