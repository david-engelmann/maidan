"""A coding agent on Maidan's waiter loop: claim a task, run the agent, hand back a patch.

Maidan's side is complete: a fenced lease kept alive for as long as the work
runs, the result on the thread, the patch as a content-addressed artifact, the
claim released however the run ends. The coding itself is `AGENT_COMMAND`, run
in an empty working directory with the task in `$MAIDAN_TASK`; whatever it
leaves in `$MAIDAN_OUTPUT_DIR` is uploaded and attached to the thread.

With no `AGENT_COMMAND` a stand-in runs instead. It writes a patch that adds a
file recording the task. It is not an LLM and does not pretend to be one.

    AGENT_COMMAND='claude -p "$MAIDAN_TASK" && git diff > "$MAIDAN_OUTPUT_DIR/change.patch"'
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile
import time

from maidan_http import Claim, Maidan, claim_next, from_creds

LEASE_SECS = int(os.environ.get("LEASE_SECS", "120"))
POLL_SECS = float(os.environ.get("POLL_SECS", "2"))
RUN_TIMEOUT_SECS = int(os.environ.get("RUN_TIMEOUT_SECS", "1800"))


def stand_in(task: str, out: pathlib.Path) -> tuple[int, str]:
    lines = ["# Task", "", *task.splitlines()]
    patch = "\n".join(
        [
            "--- /dev/null",
            "+++ b/TASK.md",
            f"@@ -0,0 +1,{len(lines)} @@",
            *(f"+{line}" for line in lines),
            "",
        ]
    )
    (out / "change.patch").write_text(patch)
    return 0, "stand-in: wrote change.patch (no LLM; set AGENT_COMMAND to run a real agent)"


def run_agent(claim: Claim, task: str, work: pathlib.Path) -> tuple[int, str]:
    out = work / "out"
    out.mkdir()
    command = os.environ.get("AGENT_COMMAND", "").strip()
    if not command:
        return stand_in(task, out)
    env = {
        **os.environ,
        "MAIDAN_TASK": task,
        "MAIDAN_THREAD_ID": claim.thread_id,
        "MAIDAN_OUTPUT_DIR": str(out),
    }
    try:
        done = subprocess.run(
            command,
            shell=True,
            cwd=work,
            env=env,
            capture_output=True,
            text=True,
            timeout=RUN_TIMEOUT_SECS,
        )
    except subprocess.TimeoutExpired:
        return 124, f"AGENT_COMMAND timed out after {RUN_TIMEOUT_SECS}s"
    tail = (done.stdout + done.stderr).strip().splitlines()[-20:]
    return done.returncode, "\n".join(tail)


def upload(maidan: Maidan, path: pathlib.Path) -> str:
    kind = "code_dump" if path.suffix in {".patch", ".diff"} else "attachment"
    artifact = maidan.call(
        "POST",
        f"/artifacts?kind={kind}&mime_type=text/plain",
        raw=path.read_bytes(),
        content_type="application/octet-stream",
    )
    return artifact["sha256"]


def work_one(maidan: Maidan, claim: Claim) -> None:
    with claim:
        task = claim.task()
        print(f"coding-agent: claimed {claim.thread_id}: {task}", flush=True)
        with tempfile.TemporaryDirectory() as tmp:
            work = pathlib.Path(tmp)
            code, log = run_agent(claim, task, work)
            shas = [upload(maidan, p) for p in sorted((work / "out").iterdir()) if p.is_file()]
        status = "done" if code == 0 else "failed"
        claim.post(f"{status} (exit {code})\n\n{log}", artifacts=shas)
        claim.finish({"status": status, "exit_code": code, "artifacts": shas})
        print(f"coding-agent: {status} {claim.thread_id}, artifacts {shas}", flush=True)


def main() -> int:
    maidan, creds = from_creds()
    once = "--once" in sys.argv
    while True:
        claim = claim_next(maidan, creds["channel_id"], LEASE_SECS)
        if claim is None:
            if once:
                return 0
            time.sleep(POLL_SECS)
            continue
        work_one(maidan, claim)
        if once:
            return 0


if __name__ == "__main__":
    raise SystemExit(main())
