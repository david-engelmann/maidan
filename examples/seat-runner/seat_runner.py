#!/usr/bin/env python3
"""Keep N session seats claiming Maidan work, one task per harness run.

Each run, per seat:

1. Host check, here and not in the harness: the 15-minute load is under
   `host.max_load15` and the checkout's disk has `host.min_free_gb` free. A
   harness sandbox cannot see either (Codex's `workspace-write` denies `ps`).
2. Queue poll: `GET /workspaces/{wid}/queue-depth` with this seat's `engine`
   and `repo`. Nothing ready, no run: a harness run costs real money.
3. Preflight: the toolchain's version and the repository's own narrow check,
   under `capped`, outside the sandbox. A failure claims nothing.
4. Claim: `POST /workspaces/{wid}/threads/claim-next` with the same filters,
   then acknowledge. `null` ends the run (with `--once`, exit 0).
5. Harness: the whole harness process under `capped --time S --mem-gb G`,
   told the thread and the lease so it starts at the runbook's step 3.
6. Release: always, whatever the harness did (a timeout, a crash, SIGTERM).
   A claim the harness already released answers 409, which is fine.

One JSON log line per run on stdout. SIGTERM or SIGINT stops every seat,
kills the running harnesses and releases their claims.

    seat_runner.py --config seats.toml [--once]
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

# The models each harness may run. A model outside its harness's list is
# refused at startup, so a typo or a costlier model never reaches a seat.
MODEL_ALLOWLIST: dict[str, frozenset[str]] = {
    "codex": frozenset({"gpt-5.6-sol"}),
}

ENGINE = "session"


class ConfigError(Exception):
    pass


@dataclass(frozen=True)
class Repo:
    repo: str
    checkout: Path
    preflight: list[list[str]]
    writable_roots: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class Config:
    url: str
    token: str
    workspace_id: str
    seats: int
    harness: str
    harness_bin: str
    model: str
    prompt: str
    capped: str
    run_secs: int
    run_mem_gb: float
    preflight_secs: int
    preflight_mem_gb: float
    lease_secs: int
    max_load15: float
    min_free_gb: float
    idle_secs: float
    repos: list[Repo]


def load_config(path: Path, env: dict[str, str]) -> Config:
    raw = tomllib.loads(path.read_text())
    maidan = raw.get("maidan", {})
    harness = raw.get("harness", {})
    host = raw.get("host", {})
    caps = raw.get("caps", {})
    name = harness.get("name", "codex")
    model = harness.get("model", "")
    allowed = MODEL_ALLOWLIST.get(name)
    if allowed is None:
        raise ConfigError(f"harness {name!r} is not supported ({sorted(MODEL_ALLOWLIST)})")
    if model not in allowed:
        raise ConfigError(f"model {model!r} is not allowed for {name} ({sorted(allowed)})")
    token_env = maidan.get("token_env", "MAIDAN_TOKEN")
    token = env.get(token_env, "")
    if not token:
        raise ConfigError(f"no seat token in ${token_env}")
    repos = [
        Repo(
            repo=r["repo"],
            checkout=Path(r["checkout"]).expanduser(),
            preflight=[list(c) for c in r.get("preflight", [])],
            writable_roots=[str(Path(w).expanduser()) for w in r.get("writable_roots", [])],
        )
        for r in raw.get("repos", [])
    ]
    if not repos:
        raise ConfigError("no [[repos]] configured")
    for r in repos:
        if not r.preflight:
            raise ConfigError(f"{r.repo}: a preflight (toolchain version and narrow check) is required")
    seats = int(raw.get("seats", 1))
    if seats < 1:
        raise ConfigError("seats must be at least 1")
    return Config(
        url=maidan.get("url", "http://127.0.0.1:8080").rstrip("/"),
        token=token,
        workspace_id=maidan["workspace_id"],
        seats=seats,
        harness=name,
        harness_bin=harness.get("bin", name),
        model=model,
        prompt=str(Path(harness.get("prompt", Path(__file__).with_name("prompt.md"))).expanduser()),
        capped=str(Path(caps.get("capped", "capped")).expanduser()),
        run_secs=int(caps.get("run_secs", 5400)),
        run_mem_gb=float(caps.get("run_mem_gb", 48)),
        preflight_secs=int(caps.get("preflight_secs", 900)),
        preflight_mem_gb=float(caps.get("preflight_mem_gb", 16)),
        lease_secs=int(maidan.get("lease_secs", 3600)),
        max_load15=float(host.get("max_load15", 130)),
        min_free_gb=float(host.get("min_free_gb", 65)),
        idle_secs=float(raw.get("idle_secs", 60)),
        repos=repos,
    )


class Maidan:
    def __init__(self, url: str, token: str) -> None:
        self.url = url
        self.token = token

    def call(self, method: str, path: str, body: object | None = None) -> tuple[int, object]:
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, data=data, method=method)
        req.add_header("Authorization", f"Bearer {self.token}")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=60) as res:
                text = res.read().decode()
                return res.status, json.loads(text) if text else None
        except urllib.error.HTTPError as e:
            text = e.read().decode()
            try:
                return e.code, json.loads(text)
            except ValueError:
                return e.code, text


def harness_command(cfg: Config, repo: Repo, thread_id: str, lease_id: str) -> list[str]:
    prompt = Path(cfg.prompt).read_text()
    prompt += (
        f"\n\nThis run's claim is already taken and acknowledged: thread_id "
        f"`{thread_id}`, claim_lease_id `{lease_id}`, repo `{repo.repo}`. "
        "Skip the runbook's steps 0 to 2 (preflight, claim, acknowledge) and start at step 3.\n"
    )
    cmd = [
        cfg.harness_bin, "exec",
        "-m", cfg.model,
        "--sandbox", "workspace-write",
        "--ephemeral",
        "-c", 'mcp_servers.maidan.default_tools_approval_mode="approve"',
    ]
    if repo.writable_roots:
        cmd += ["-c", "sandbox_workspace_write.writable_roots=" + json.dumps(repo.writable_roots)]
    return cmd + [prompt]


def descendants(pid: int) -> list[int]:
    """Every process below `pid`, from `ps` (run here, outside any sandbox)."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid="], capture_output=True, text=True).stdout
    children: dict[int, list[int]] = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 2:
            children.setdefault(int(parts[1]), []).append(int(parts[0]))
    found, todo = [], [pid]
    while todo:
        for child in children.get(todo.pop(), []):
            found.append(child)
            todo.append(child)
    return found


def kill_tree(pid: int) -> None:
    """SIGTERM `pid`, everything below it and their process groups. `capped`
    starts its command in a session of its own and does not forward a
    signal, so killing `capped` alone would leave the harness running."""
    pids = [pid, *descendants(pid)]
    groups = set()
    for p in pids:
        try:
            groups.add(os.getpgid(p))
        except (ProcessLookupError, PermissionError):
            pass
    groups.discard(os.getpgrp())
    for g in groups:
        try:
            os.killpg(g, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
    for p in pids:
        try:
            os.kill(p, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass


def capped(cfg: Config, secs: int, mem_gb: float, cmd: list[str]) -> list[str]:
    return [cfg.capped, "--time", str(secs), "--mem-gb", str(mem_gb), "--", *cmd]


class Runner:
    def __init__(self, cfg: Config, out=sys.stdout) -> None:
        self.cfg = cfg
        self.api = Maidan(cfg.url, cfg.token)
        self.stop = threading.Event()
        self.out = out
        self.lock = threading.Lock()
        self.running: dict[int, subprocess.Popen] = {}

    def log(self, **fields: object) -> None:
        line = json.dumps({"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), **fields})
        with self.lock:
            print(line, file=self.out, flush=True)

    def host_problem(self, repo: Repo) -> str | None:
        load15 = os.getloadavg()[2]
        if load15 >= self.cfg.max_load15:
            return f"15-min load {load15:.1f} >= {self.cfg.max_load15}"
        free_gb = shutil.disk_usage(repo.checkout).free / 1024**3
        if free_gb < self.cfg.min_free_gb:
            return f"{free_gb:.0f} GB free < {self.cfg.min_free_gb}"
        return None

    def ready(self, repo: Repo) -> int | None:
        q = urllib.parse.urlencode({"engine": ENGINE, "repo": repo.repo})
        status, body = self.api.call("GET", f"/workspaces/{self.cfg.workspace_id}/queue-depth?{q}")
        if status != 200 or not isinstance(body, dict):
            return None
        return int(body.get("ready", 0))

    def run_process(self, seat: int, cmd: list[str], cwd: Path) -> int:
        proc = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.DEVNULL, start_new_session=True)
        with self.lock:
            self.running[seat] = proc
        try:
            return proc.wait()
        finally:
            with self.lock:
                self.running.pop(seat, None)

    def preflight(self, seat: int, repo: Repo) -> str | None:
        for cmd in repo.preflight:
            code = self.run_process(
                seat, capped(self.cfg, self.cfg.preflight_secs, self.cfg.preflight_mem_gb, cmd), repo.checkout
            )
            if code != 0:
                return f"{' '.join(cmd)} exited {code}"
        return None

    def run_once(self, seat: int, repo: Repo) -> str:
        """One run for one seat; returns the outcome it logged."""
        started = time.monotonic()
        base = {"seat": seat, "repo": repo.repo}

        def done(outcome: str, **more: object) -> str:
            self.log(**base, outcome=outcome, secs=round(time.monotonic() - started, 1), **more)
            return outcome

        problem = self.host_problem(repo)
        if problem:
            return done("host_busy", detail=problem)
        ready = self.ready(repo)
        if ready is None:
            return done("queue_unreachable")
        if ready == 0:
            return done("no_work", ready=0)
        problem = self.preflight(seat, repo)
        if problem:
            return done("preflight_failed", detail=problem)
        if self.stop.is_set():
            return done("stopped")
        status, claim = self.api.call(
            "POST",
            f"/workspaces/{self.cfg.workspace_id}/threads/claim-next",
            {"engine": ENGINE, "repo": repo.repo, "lease_secs": self.cfg.lease_secs},
        )
        if status != 200:
            return done("claim_failed", status=status)
        if claim is None:
            return done("no_work", ready=ready)
        thread_id, lease_id = claim["id"], claim["claim_lease_id"]
        try:
            self.api.call("POST", f"/threads/{thread_id}/claim/acknowledge", {"claim_lease_id": lease_id})
            code = self.run_process(
                seat,
                capped(self.cfg, self.cfg.run_secs, self.cfg.run_mem_gb,
                       harness_command(self.cfg, repo, thread_id, lease_id)),
                repo.checkout,
            )
        finally:
            released, _ = self.api.call("POST", f"/threads/{thread_id}/claim/release", {"claim_lease_id": lease_id})
        outcome = "stopped" if self.stop.is_set() else {0: "ran", 124: "timed_out", 137: "out_of_memory"}.get(code, "failed")
        return done(outcome, thread=thread_id, exit=code, release=released)

    def seat_loop(self, seat: int, once: bool) -> None:
        repos = self.cfg.repos
        turn = seat
        while not self.stop.is_set():
            repo = repos[turn % len(repos)]
            turn += 1
            outcome = self.run_once(seat, repo)
            if once:
                return
            if outcome != "ran":
                self.stop.wait(self.cfg.idle_secs)

    def shutdown(self, *_: object) -> None:
        self.stop.set()
        with self.lock:
            procs = list(self.running.values())
        for proc in procs:
            kill_tree(proc.pid)

    def run(self, once: bool) -> int:
        threads = [
            threading.Thread(target=self.seat_loop, args=(seat, once), daemon=True)
            for seat in range(self.cfg.seats)
        ]
        for t in threads:
            t.start()
        for t in threads:
            while t.is_alive():
                t.join(0.5)
        return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--once", action="store_true", help="one run per seat, then exit")
    args = parser.parse_args(argv)
    try:
        cfg = load_config(args.config, dict(os.environ))
    except (ConfigError, KeyError, OSError, tomllib.TOMLDecodeError) as e:
        print(f"seat-runner: {e}", file=sys.stderr)
        return 2
    runner = Runner(cfg)
    signal.signal(signal.SIGTERM, runner.shutdown)
    signal.signal(signal.SIGINT, runner.shutdown)
    return runner.run(args.once)


if __name__ == "__main__":
    sys.exit(main())
