#!/usr/bin/env python3
"""Keep N session seats claiming Maidan work, one task per harness run.

Each run, per seat (README.md has the detail):

1. Host check, here and not in the harness sandbox: 15-minute load and free
   disk against `[host]`.
2. Queue poll: `GET /workspaces/{wid}/queue-depth` with this seat's engine and
   repo. Nothing ready, no run.
3. Preflight: the repo's own commands under `capped`, without the seat token
   in their environment. A failure claims nothing.
4. Claim (`claim-next`) and acknowledge. `null` ends the run; a failed
   acknowledge releases the claim and runs no harness.
5. Harness: the whole harness process under `capped`, told the thread and
   lease so it skips the runbook's "Claim" and "Acknowledge" sections.
6. Release, always, and only once every process the run started is dead.

One JSON log line per run on stdout. SIGTERM or SIGINT stops every seat:
running harnesses get SIGTERM, then SIGKILL after a grace period, then their
claims are released.

    seat_runner.py --config seats.toml [--once]
"""
from __future__ import annotations

import argparse
import ipaddress
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

ENGINE = "session"
# `Maidan.call`'s status when the server could not be reached at all.
UNREACHABLE = 0
# `capped`'s exit codes: the time cap, the memory cap.
CAPPED_TIME, CAPPED_MEMORY = 124, 137


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
    token_env: str
    secret_env: frozenset[str]
    workspace_id: str
    seats: int
    harness_bin: str
    model: str
    pass_token: bool
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
    kill_grace_secs: float
    release_attempts: int
    release_backoff_secs: float
    repos: list[Repo]


def is_loopback(url: str) -> bool:
    host = urllib.parse.urlparse(url).hostname or ""
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def load_config(path: Path, env: dict[str, str]) -> Config:
    raw = tomllib.loads(path.read_text())
    maidan = raw.get("maidan", {})
    harness = raw.get("harness", {})
    host = raw.get("host", {})
    caps = raw.get("caps", {})
    if harness.get("name", "codex") != "codex":
        raise ConfigError("only the codex harness is supported")
    models = harness.get("models")
    if not isinstance(models, list) or not models:
        raise ConfigError("[harness] models must list the models this runner may start")
    model = harness.get("model", "")
    if model not in models:
        raise ConfigError(f"model {model!r} is not in [harness] models {models}")
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
    url = maidan.get("url", "http://127.0.0.1:8080").rstrip("/")
    if not url.startswith("https://") and not is_loopback(url):
        print(f"seat-runner: warning: {url} is not https; the seat token crosses the network in clear",
              file=sys.stderr)
    return Config(
        url=url,
        token=token,
        token_env=token_env,
        secret_env=frozenset({token_env, *maidan.get("secret_env", [])}),
        workspace_id=maidan["workspace_id"],
        seats=seats,
        harness_bin=harness.get("bin", "codex"),
        model=model,
        pass_token=bool(harness.get("pass_token", True)),
        prompt=str(Path(harness.get("prompt", Path(__file__).with_name("prompt.md"))).expanduser()),
        capped=str(Path(caps.get("capped", "scripts/capped")).expanduser()),
        run_secs=int(caps.get("run_secs", 3600)),
        run_mem_gb=float(caps.get("run_mem_gb", 8)),
        preflight_secs=int(caps.get("preflight_secs", 600)),
        preflight_mem_gb=float(caps.get("preflight_mem_gb", 4)),
        lease_secs=int(maidan.get("lease_secs", 3600)),
        max_load15=float(host.get("max_load15", os.cpu_count() or 1)),
        min_free_gb=float(host.get("min_free_gb", 5)),
        idle_secs=float(raw.get("idle_secs", 60)),
        kill_grace_secs=float(raw.get("kill_grace_secs", 10)),
        release_attempts=int(maidan.get("release_attempts", 4)),
        release_backoff_secs=float(maidan.get("release_backoff_secs", 2)),
        repos=repos,
    )


class Maidan:
    def __init__(self, url: str, token: str) -> None:
        self.url = url
        self.token = token

    def call(self, method: str, path: str, body: object | None = None) -> tuple[int, object]:
        """`(status, body)`; `(UNREACHABLE, reason)` when there is no answer."""
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
            text = e.read().decode(errors="replace")
            try:
                return e.code, json.loads(text)
            except ValueError:
                return e.code, text
        except (urllib.error.URLError, TimeoutError, OSError, ValueError) as e:
            return UNREACHABLE, str(e)


def harness_command(cfg: Config, repo: Repo, thread_id: str, lease_id: str) -> list[str]:
    prompt = Path(cfg.prompt).read_text()
    prompt += (
        f"\n\nThis run's claim is already taken and acknowledged: thread_id "
        f"`{thread_id}`, claim_lease_id `{lease_id}`, repo `{repo.repo}`. Skip the "
        "runbook's sections \"0b. Before every claim: the toolchain\", \"1. Claim\" and "
        "\"2. Acknowledge\", and start at \"3. Read the task\".\n"
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


def capped(cfg: Config, secs: int, mem_gb: float, cmd: list[str]) -> list[str]:
    return [cfg.capped, "--time", str(secs), "--mem-gb", str(mem_gb), "--", *cmd]


def process_table() -> list[tuple[int, int, int]]:
    """`(pid, ppid, pgid)` of every process, from `ps` (run here, outside any
    sandbox)."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,pgid="], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3:
            rows.append((int(parts[0]), int(parts[1]), int(parts[2])))
    return rows


class Tree:
    """The pids and process groups one child has started, gathered while it
    runs: once `capped` exits, its command is reparented and no longer found
    under it, so the set is kept as it grows."""

    def __init__(self, root: int) -> None:
        self.root = root
        self.pids: set[int] = {root}
        self.groups: set[int] = set()

    def refresh(self) -> None:
        table = process_table()
        children: dict[int, list[int]] = {}
        group_of = {}
        for pid, ppid, pgid in table:
            children.setdefault(ppid, []).append(pid)
            group_of[pid] = pgid
        todo = [self.root]
        while todo:
            pid = todo.pop()
            if pid in group_of:
                self.groups.add(group_of[pid])
            for child in children.get(pid, []):
                if child not in self.pids:
                    self.pids.add(child)
                todo.append(child)
        self.groups.discard(os.getpgrp())

    def alive(self) -> bool:
        live = {pid for pid, _, _ in process_table()}
        return bool(self.pids & live)

    def signal(self, sig: int) -> None:
        for g in self.groups:
            try:
                os.killpg(g, sig)
            except (ProcessLookupError, PermissionError):
                pass
        for p in self.pids:
            try:
                os.kill(p, sig)
            except (ProcessLookupError, PermissionError):
                pass

    def kill(self, grace: float) -> None:
        """SIGTERM, up to `grace` seconds, then SIGKILL whatever is left."""
        self.refresh()
        self.signal(signal.SIGTERM)
        deadline = time.monotonic() + grace
        while time.monotonic() < deadline and self.alive():
            time.sleep(0.2)
        if self.alive():
            self.signal(signal.SIGKILL)


class Runner:
    def __init__(self, cfg: Config, out=sys.stdout) -> None:
        self.cfg = cfg
        self.api = Maidan(cfg.url, cfg.token)
        self.stop = threading.Event()
        self.out = out
        self.lock = threading.Lock()
        self.running: dict[int, Tree] = {}

    def log(self, **fields: object) -> None:
        line = json.dumps({"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), **fields})
        with self.lock:
            print(line, file=self.out, flush=True)

    def env(self, with_token: bool) -> dict[str, str]:
        env = {k: v for k, v in os.environ.items() if k not in self.cfg.secret_env}
        if with_token:
            env[self.cfg.token_env] = self.cfg.token
        return env

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

    def run_process(self, seat: int, cmd: list[str], cwd: Path, env: dict[str, str]) -> int:
        """Run `cmd` to its end. Returns only once everything it started is
        dead, so a claim released after it is not still being worked."""
        proc = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.DEVNULL, start_new_session=True)
        tree = Tree(proc.pid)
        with self.lock:
            self.running[seat] = tree
        try:
            if self.stop.is_set():
                tree.kill(self.cfg.kill_grace_secs)
            while True:
                try:
                    code = proc.wait(timeout=0.5)
                    break
                except subprocess.TimeoutExpired:
                    tree.refresh()
            tree.kill(self.cfg.kill_grace_secs)
            return code
        finally:
            with self.lock:
                self.running.pop(seat, None)

    def preflight(self, seat: int, repo: Repo) -> str | None:
        for cmd in repo.preflight:
            code = self.run_process(
                seat,
                capped(self.cfg, self.cfg.preflight_secs, self.cfg.preflight_mem_gb, cmd),
                repo.checkout,
                self.env(with_token=False),
            )
            if code != 0:
                return f"{' '.join(cmd)} exited {code}"
        return None

    def release(self, thread_id: str, lease_id: str) -> int:
        """Release, retrying an unreachable server with backoff. 2xx or 409
        (the harness already released, or the lease lapsed) is final."""
        status = UNREACHABLE
        for attempt in range(self.cfg.release_attempts):
            status, _ = self.api.call("POST", f"/threads/{thread_id}/claim/release",
                                      {"claim_lease_id": lease_id})
            if status != UNREACHABLE and status < 500:
                return status
            time.sleep(self.cfg.release_backoff_secs * 2**attempt)
        return status

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
        if not (isinstance(claim, dict) and isinstance(claim.get("id"), str)
                and isinstance(claim.get("claim_lease_id"), str)):
            return done("claim_failed", status=status, detail="claim answer without id and claim_lease_id")
        thread_id, lease_id = claim["id"], claim["claim_lease_id"]
        outcome, code = "stopped", None
        try:
            ack, _ = self.api.call("POST", f"/threads/{thread_id}/claim/acknowledge",
                                   {"claim_lease_id": lease_id})
            if not 200 <= ack < 300:
                outcome = "ack_failed"
            elif not self.stop.is_set():
                code = self.run_process(
                    seat,
                    capped(self.cfg, self.cfg.run_secs, self.cfg.run_mem_gb,
                           harness_command(self.cfg, repo, thread_id, lease_id)),
                    repo.checkout,
                    self.env(with_token=self.cfg.pass_token),
                )
                outcome = "stopped" if self.stop.is_set() else {
                    0: "ran", CAPPED_TIME: "timed_out", CAPPED_MEMORY: "out_of_memory",
                }.get(code, "failed")
        finally:
            released = self.release(thread_id, lease_id)
        return done(outcome, thread=thread_id, exit=code, release=released)

    def seat_loop(self, seat: int, once: bool) -> None:
        repos = self.cfg.repos
        turn = seat
        while not self.stop.is_set():
            repo = repos[turn % len(repos)]
            turn += 1
            try:
                outcome = self.run_once(seat, repo)
            except Exception as e:  # a seat logs and carries on; it never dies silently
                outcome = "error"
                self.log(seat=seat, repo=repo.repo, outcome=outcome, detail=f"{type(e).__name__}: {e}")
            if once:
                return
            if outcome != "ran":
                self.stop.wait(self.cfg.idle_secs)

    def shutdown(self, *_: object) -> None:
        self.stop.set()
        with self.lock:
            trees = list(self.running.values())
        for tree in trees:
            threading.Thread(target=tree.kill, args=(self.cfg.kill_grace_secs,), daemon=True).start()

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
