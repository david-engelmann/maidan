#!/usr/bin/env python3
"""Keep N session seats claiming Maidan work, one task per harness run.

Each run, per seat (README.md has the detail):

1. Host check, here and not in the harness sandbox: 15-minute load and free
   disk against `[host]`.
2. Queue poll: `GET /workspaces/{wid}/queue-depth` with this seat's engine and
   repo. Nothing ready, no run.
3. Preflight: the repo's own commands under `capped`, without the seat token
   in their environment. A failure claims nothing.
4. Claim (`claim-next`) and acknowledge, each with one Idempotency-Key
   across its retries, so a claim whose answer was lost is answered again
   rather than taken twice. `null` ends the run; a failed acknowledge
   releases the claim and runs no harness.
5. Harness (`codex`, `claude` or `gemini`): the whole harness process under
   `capped`, told the thread and lease so it skips the runbook's "Claim" and
   "Acknowledge" sections, and told its own provider and family so its usage
   evidence and its own-family decline name the model that really ran.
6. Release, always, and only once every process the run started is dead.

One JSON log line per run on stdout. SIGTERM or SIGINT stops every seat:
running harnesses get SIGTERM, then SIGKILL after a grace period, then their
claims are released.

    seat_runner.py --config seats.toml [--once]
"""
from __future__ import annotations

import argparse
import http.client
import ipaddress
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass, field
from pathlib import Path

ENGINE = "session"
# `Maidan.call`'s status when the server could not be reached at all.
UNREACHABLE = 0
# `capped`'s exit codes: the time cap, the memory cap.
CAPPED_TIME, CAPPED_MEMORY = 124, 137
# Answers worth another try with the same Idempotency-Key: the server did not
# do it, or may not have, and says so.
RETRY_STATUSES = frozenset({408, 429, 500, 502, 503, 504})
IN_FLIGHT = "https://maidan.world/problems/idempotency-key-in-flight"
# The longest a Retry-After holds a seat between tries.
MAX_RETRY_AFTER_SECS = 60.0

# The model families a review is routed between, each with the words that name
# it in a usage record's provider or model and the bare model prefixes that do.
# A twin of `PROVIDER_FAMILIES` in crates/maidan-types/src/task_attributes.rs
# (the tests compare the two): the seat refuses to start when Maidan would read
# its provider and model as another family than the one it declares, because a
# wrong family defeats second-family review.
PROVIDER_FAMILIES: tuple[tuple[str, tuple[str, ...], tuple[str, ...]], ...] = (
    ("anthropic", ("anthropic", "claude"), ()),
    ("openai", ("openai", "gpt", "codex"), ("o1", "o3", "o4")),
    ("google", ("google", "gemini", "vertex"), ()),
    ("xai", ("xai", "grok"), ()),
    ("deepseek", ("deepseek",), ()),
    ("zhipu", ("zhipu", "z.ai", "bigmodel", "glm"), ()),
)


def provider_family(provider: str, model: str) -> str | None:
    """The family Maidan reads from a usage record's provider and model."""
    named = f"{provider} {model}".lower()
    model = model.lower()
    for family, words, prefixes in PROVIDER_FAMILIES:
        if any(word in named for word in words) or any(
                model == prefix or model.startswith(prefix + "-") for prefix in prefixes):
            return family
    return None


@dataclass(frozen=True)
class HarnessKind:
    bin: str
    provider: str
    # The `harness` a run names in its usage evidence (the runbook's table).
    evidence: str
    claimant_dir: str


HARNESSES = {
    "codex": HarnessKind("codex", "openai", "codex", "~/.codex/maidan-claimant"),
    "claude": HarnessKind("claude", "anthropic", "claude-code", "~/.claude/skills/maidan-claimant"),
    "gemini": HarnessKind("gemini", "google", "gemini-cli", "~/.gemini/maidan-claimant"),
    # Z.ai's own coding agent, the GLM coding plan's native harness.
    "zcode": HarnessKind("zcode", "z.ai", "zcode", "~/.zcode/maidan-claimant"),
}
# ZCode's model API protocols, and the GLM coding plan's Anthropic-compatible
# endpoint as the default.
ZCODE_API_TYPES = ("anthropic-messages", "openai-chat-completions", "openai-responses")
ZCODE_BASE_URL = "https://api.z.ai/api/anthropic"
# ZCode reads its runtime config (where MCP servers live) only from
# ~/.zcode/cli/config.json, so a zcode run gets its own HOME. These keep the
# tools a run uses on the real home's state, unless already set.
HOME_PINNED = {"CARGO_HOME": ".cargo", "RUSTUP_HOME": ".rustup",
               "GIT_CONFIG_GLOBAL": ".gitconfig", "XDG_CONFIG_HOME": ".config"}
# The endpoints a runner-built MCP config may name. The runbooks call tools
# outside the worker and reviewer profiles (list_messages, get_thread_result,
# list_result_deliveries, register_thread_pull; renew and release for a
# review), so the full catalog is the default.
MCP_PATHS = ("/mcp/streamable", "/mcp/worker", "/mcp/reviewer")
# Claude Code's credentials. A seat pointed at another endpoint must not hand
# an Anthropic key from the runner's environment to that endpoint.
ANTHROPIC_CREDENTIALS = ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN")
# A literal `env` value under a name like these would put a secret in the file.
SECRET_NAME = re.compile(r"KEY|TOKEN|SECRET|PASSWORD|AUTH|CREDENTIAL", re.IGNORECASE)
ENV_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
# Gemini's macOS Seatbelt profiles have five slots for --include-directories.
GEMINI_MAX_INCLUDE_DIRS = 5
# The placeholders a prompt file may use.
PROMPT_FIELDS = re.compile(r"\{(provider|harness|family|model|claimant_dir)\}")
# Claude Code runs the headersHelper in a shell and reads one JSON object of
# headers from it. It reads the seat token from the environment, so the token
# is on no command line and in no file.
HEADERS_HELPER = ('import json, os, sys; '
                  'print(json.dumps({"Authorization": "Bearer " + os.environ[sys.argv[1]]}))')


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
    harness: str
    # The harness's argv prefix: a binary, or e.g. ["node", "/abs/zcode.cjs"].
    harness_bin: list[str]
    model: str
    # Who serves the model and the family Maidan reads from it: what the
    # prompts put in the usage evidence and the own-family decline.
    provider: str
    family: str
    evidence_harness: str
    claimant_dir: str
    pass_token: bool
    # Literal variables for the harness, and variables copied into it from
    # the runner's environment (name -> value), e.g. GLM's endpoint and key.
    harness_env: dict[str, str]
    harness_secrets: dict[str, str]
    # Variables the runner's environment must not pass to the harness.
    harness_drop_env: frozenset[str]
    mcp_url: str
    allowed_domains: list[str]
    state_dir: Path
    # zcode: the model API it calls and the key for it, read from a named
    # variable and written only to the run's private provider file.
    api_type: str
    api_base_url: str
    api_key: str
    prompt: str
    # A prompt per task kind (`attributes.kind` on the claimed thread), e.g. a
    # review thread runs the red-team prompt; any other kind runs `prompt`.
    prompts: dict[str, str]
    capped: str
    run_secs: int
    run_mem_gb: float
    preflight_secs: int
    preflight_mem_gb: float
    lease_secs: int
    timeout_secs: float
    claim_attempts: int
    claim_backoff_secs: float
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


def harness_variables(harness: dict, token_env: str,
                      env: dict[str, str]) -> tuple[dict[str, str], dict[str, str], set[str]]:
    """`[harness] env` (literal values) and `env_from` (a variable for the
    harness, filled from a named variable of the runner's environment):
    `(literal, secret values, the source names)`. A secret is only ever
    read from the environment, never from the file."""
    literal = harness.get("env", {})
    copied = harness.get("env_from", {})
    if not isinstance(literal, dict) or not isinstance(copied, dict):
        raise ConfigError("[harness] env and env_from map variable names to strings")
    for var, value in literal.items():
        if not ENV_NAME.match(var) or not isinstance(value, str):
            raise ConfigError(f"[harness] env {var!r} must be a variable name with a string value")
        if SECRET_NAME.search(var):
            raise ConfigError(f"[harness] env {var}: a secret never goes in the file; "
                              f"name the variable that holds it in env_from ({var} = \"SOME_VAR\")")
    secrets: dict[str, str] = {}
    for var, source in copied.items():
        if not ENV_NAME.match(var) or not isinstance(source, str) or not ENV_NAME.match(source):
            raise ConfigError(f"[harness] env_from {var!r} must name a variable to read it from")
        value = env.get(source, "")
        if not value:
            raise ConfigError(f"[harness] env_from {var}: ${source} is not set")
        secrets[var] = value
    for var in {*literal, *copied} & {token_env, "GEMINI_CLI_SYSTEM_SETTINGS_PATH"}:
        raise ConfigError(f"[harness] env: {var} is set by the runner")
    return dict(literal), secrets, set(copied.values())


def is_anthropic_url(url: str) -> bool:
    host = (urllib.parse.urlparse(url).hostname or "").lower()
    return host == "anthropic.com" or host.endswith(".anthropic.com")


def claude_endpoint_check(provider: str, family: str, env: dict[str, str],
                          secrets: dict[str, str]) -> set[str]:
    """Refuse a Claude Code seat whose endpoint and provider disagree, and
    return the Anthropic credentials the harness must not inherit. GLM's
    coding plan is Claude Code pointed at z.ai's Anthropic-compatible URL; a
    seat that leaves its provider `anthropic` there would report GLM's work
    as Anthropic's and review Anthropic's changes."""
    base = env.get("ANTHROPIC_BASE_URL", "")
    if base:
        endpoint_check(base, family)
    if base and not is_anthropic_url(base):
        if family == "anthropic":
            host = urllib.parse.urlparse(base).hostname or base
            raise ConfigError(f"ANTHROPIC_BASE_URL points claude at {host}, not Anthropic, but the "
                              f"provider is {provider!r}: set [harness] provider to the endpoint's vendor "
                              "(\"z.ai\" for GLM's coding plan)")
        # The endpoint's own key comes from env_from; an Anthropic key in the
        # runner's environment would otherwise go to a third party.
        return {var for var in ANTHROPIC_CREDENTIALS if var not in secrets}
    if family != "anthropic":
        raise ConfigError(f"provider {provider!r} is not Anthropic, but claude talks to Anthropic: "
                          "set ANTHROPIC_BASE_URL in [harness] env to the provider's endpoint")
    return set()


def harness_argv0(value: object) -> list[str]:
    if isinstance(value, str) and value:
        return [value]
    if isinstance(value, list) and value and all(isinstance(v, str) and v for v in value):
        return list(value)
    raise ConfigError("[harness] bin is a program, or a list such as [\"node\", \"/abs/zcode.cjs\"]")


def endpoint_check(url: str, family: str) -> None:
    """A model endpoint whose host names a family (api.z.ai, *.anthropic.com,
    api.openai.com) must be the seat's own family."""
    host = (urllib.parse.urlparse(url).hostname or "").lower()
    named = provider_family(host, "")
    if named is not None and named != family:
        raise ConfigError(f"the model endpoint {host} is {named}'s, but this seat's family is {family!r}: "
                          "set [harness] provider to the endpoint's vendor")


def zcode_api(harness: dict, family: str, env: dict[str, str]) -> tuple[str, str, str, str]:
    """`(api_type, base_url, key, key variable)` for a zcode seat's provider
    file. ZCode takes the key only as plaintext in that file, so the key is
    read from the variable `api_key_env` names and nowhere else."""
    api_type = str(harness.get("api_type", ZCODE_API_TYPES[0]))
    if api_type not in ZCODE_API_TYPES:
        raise ConfigError(f"[harness] api_type {api_type!r} is not one of {list(ZCODE_API_TYPES)}")
    base_url = str(harness.get("api_base_url", ZCODE_BASE_URL))
    if not base_url.startswith("https://"):
        raise ConfigError(f"[harness] api_base_url {base_url!r}: the model key would cross the network in clear")
    endpoint_check(base_url, family)
    key_env = harness.get("api_key_env")
    if not isinstance(key_env, str) or not ENV_NAME.match(key_env):
        raise ConfigError("[harness] api_key_env must name the variable holding the model API key")
    key = env.get(key_env, "")
    if not key:
        raise ConfigError(f"[harness] api_key_env: ${key_env} is not set")
    return api_type, base_url, key, key_env


def load_config(path: Path, env: dict[str, str]) -> Config:
    raw = tomllib.loads(path.read_text())
    maidan = raw.get("maidan", {})
    harness = raw.get("harness", {})
    host = raw.get("host", {})
    caps = raw.get("caps", {})
    name = harness.get("name", "codex")
    spec = HARNESSES.get(name)
    if spec is None:
        raise ConfigError(f"[harness] name {name!r} is not one of {sorted(HARNESSES)}")
    models = harness.get("models")
    if not isinstance(models, list) or not models:
        raise ConfigError("[harness] models must list the models this runner may start")
    model = harness.get("model", "")
    if model not in models:
        raise ConfigError(f"model {model!r} is not in [harness] models {models}")
    token_env = maidan.get("token_env", "MAIDAN_TOKEN")
    if not ENV_NAME.match(token_env):
        raise ConfigError(f"[maidan] token_env {token_env!r} is not a variable name")
    token = env.get(token_env, "")
    if not token:
        raise ConfigError(f"no seat token in ${token_env}")
    pass_token = bool(harness.get("pass_token", True))
    if name in ("claude", "gemini") and not pass_token:
        raise ConfigError(f"the {name} harness's MCP config reads the seat token from ${token_env}; "
                          "pass_token = false is for codex and zcode only")
    harness_env, harness_secrets, secret_sources = harness_variables(harness, token_env, env)
    provider = str(harness.get("provider", spec.provider))
    declared = harness.get("family")
    read = provider_family(provider, model)
    if read is None:
        raise ConfigError(f"Maidan reads no model family from provider {provider!r} and model {model!r}; "
                          "set [harness] provider to the model's vendor")
    family = str(declared).lower() if declared is not None else read
    if family != read:
        raise ConfigError(f"[harness] family {family!r}, but Maidan reads provider {provider!r} and "
                          f"model {model!r} as {read!r}: a wrong family defeats second-family review")
    drop: set[str] = set()
    if name == "claude":
        drop = claude_endpoint_check(provider, family, {**env, **harness_env}, harness_secrets)
    api_type = api_base_url = api_key = ""
    if name == "zcode":
        api_type, api_base_url, api_key, key_env = zcode_api(harness, family, env)
        secret_sources.add(key_env)
    else:
        for key in ("api_type", "api_base_url", "api_key_env"):
            if key in harness:
                raise ConfigError(f"[harness] {key} is for the zcode harness")
    url = maidan.get("url", "http://127.0.0.1:8080").rstrip("/")
    mcp_path = harness.get("mcp_path")
    if mcp_path is not None and name == "codex":
        raise ConfigError("[harness] mcp_path: codex reads its MCP server from ~/.codex/config.toml")
    mcp_path = mcp_path or MCP_PATHS[0]
    if mcp_path not in MCP_PATHS:
        raise ConfigError(f"[harness] mcp_path {mcp_path!r} is not one of {list(MCP_PATHS)}")
    domains = harness.get("allowed_domains")
    if domains is not None and name != "claude":
        raise ConfigError("[harness] allowed_domains is Claude Code's sandbox network list; "
                          f"the {name} harness does not read it")
    if domains is None:
        domains = [urllib.parse.urlparse(url).hostname or "127.0.0.1"]
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
        if name == "zcode" and r.writable_roots:
            raise ConfigError(f"{r.repo}: zcode has no sandbox and writes wherever this user can, so "
                              "writable_roots would promise a limit it does not have; remove it")
        if name == "gemini" and len(r.writable_roots) > GEMINI_MAX_INCLUDE_DIRS:
            raise ConfigError(f"{r.repo}: Gemini's sandbox can make at most {GEMINI_MAX_INCLUDE_DIRS} "
                              "writable_roots writable")
    state_dir = Path(harness.get("state_dir", Path(tempfile.gettempdir()) / f"maidan-seat-runner-{os.getuid()}"))
    # Not resolved where it is kept: state_root refuses a symlinked one.
    state_dir = state_dir.expanduser()
    for r in repos:
        if state_dir.resolve().is_relative_to(r.checkout.resolve()):
            raise ConfigError(f"[harness] state_dir {state_dir} is inside {r.repo}'s checkout; "
                              "a run's private files never go there")
    prompts = {str(kind): str(Path(path).expanduser()) for kind, path in harness.get("prompts", {}).items()}
    for kind, path in prompts.items():
        if not Path(path).is_file():
            raise ConfigError(f"[harness.prompts] {kind}: no prompt file at {path}")
    seats = int(raw.get("seats", 1))
    if seats < 1:
        raise ConfigError("seats must be at least 1")
    if not url.startswith("https://") and not is_loopback(url):
        print(f"seat-runner: warning: {url} is not https; the seat token crosses the network in clear",
              file=sys.stderr)
    return Config(
        url=url,
        token=token,
        token_env=token_env,
        secret_env=frozenset({token_env, *maidan.get("secret_env", []), *secret_sources}),
        workspace_id=maidan["workspace_id"],
        seats=seats,
        harness=name,
        harness_bin=harness_argv0(harness.get("bin", spec.bin)),
        model=model,
        provider=provider,
        family=family,
        evidence_harness=spec.evidence,
        claimant_dir=str(harness.get("claimant_dir", spec.claimant_dir)),
        pass_token=pass_token,
        harness_env=harness_env,
        harness_secrets=harness_secrets,
        harness_drop_env=frozenset(drop),
        mcp_url=url + mcp_path,
        allowed_domains=[str(d) for d in domains],
        state_dir=state_dir,
        api_type=api_type,
        api_base_url=api_base_url,
        api_key=api_key,
        prompt=str(Path(harness.get("prompt", Path(__file__).with_name("prompt.md"))).expanduser()),
        prompts=prompts,
        capped=str(Path(caps.get("capped", "scripts/capped")).expanduser()),
        run_secs=int(caps.get("run_secs", 3600)),
        run_mem_gb=float(caps.get("run_mem_gb", 8)),
        preflight_secs=int(caps.get("preflight_secs", 600)),
        preflight_mem_gb=float(caps.get("preflight_mem_gb", 4)),
        lease_secs=int(maidan.get("lease_secs", 3600)),
        timeout_secs=float(maidan.get("timeout_secs", 60)),
        claim_attempts=max(1, int(maidan.get("claim_attempts", 3))),
        claim_backoff_secs=float(maidan.get("claim_backoff_secs", 2)),
        max_load15=float(host.get("max_load15", os.cpu_count() or 1)),
        min_free_gb=float(host.get("min_free_gb", 5)),
        idle_secs=float(raw.get("idle_secs", 60)),
        kill_grace_secs=float(raw.get("kill_grace_secs", 10)),
        release_attempts=max(1, int(maidan.get("release_attempts", 4))),
        release_backoff_secs=float(maidan.get("release_backoff_secs", 2)),
        repos=repos,
    )


class Maidan:
    def __init__(self, url: str, token: str, timeout: float = 60) -> None:
        self.url = url
        self.token = token
        self.timeout = timeout

    def call(self, method: str, path: str, body: object | None = None) -> tuple[int, object]:
        """`(status, body)`; `(UNREACHABLE, reason)` when there is no answer."""
        status, answer, _ = self.send(method, path, body)
        return status, answer

    def send(self, method: str, path: str, body: object | None = None,
             key: str | None = None) -> tuple[int, object, str | None]:
        """`(status, body, Retry-After)`. An answer cut short or not JSON
        counts as no answer: whether the server acted is unknown."""
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, data=data, method=method)
        req.add_header("Authorization", f"Bearer {self.token}")
        # Cloudflare refuses urllib's default agent (error 1010) in front of
        # maidan.world, so every queue check read as unreachable.
        req.add_header("User-Agent", "maidan-seat-runner/1")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if key is not None:
            req.add_header("Idempotency-Key", key)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as res:
                text = res.read().decode()
                return res.status, json.loads(text) if text else None, None
        except urllib.error.HTTPError as e:
            retry_after = e.headers.get("Retry-After") if e.headers is not None else None
            try:
                text = e.read().decode(errors="replace")
            except (http.client.HTTPException, OSError):
                text = ""
            finally:
                e.close()
            try:
                return e.code, json.loads(text), retry_after
            except ValueError:
                return e.code, text, retry_after
        # An answer cut off mid-body (IncompleteRead) is an
        # `http.client.HTTPException`, neither a URLError nor an OSError, and
        # escaped this, so the run and its release ended in a traceback.
        except (urllib.error.URLError, http.client.HTTPException, OSError, ValueError) as e:
            return UNREACHABLE, f"{type(e).__name__}: {e}", None


def retry_wait(attempt: int, backoff: float, retry_after: str | None) -> float:
    """Seconds before the next try: the server's Retry-After when it sent
    one, else `backoff` doubled per try; never more than a minute."""
    if retry_after is not None:
        try:
            return min(max(float(retry_after), 0.0), MAX_RETRY_AFTER_SECS)
        except ValueError:
            pass
    return min(backoff * 2**attempt, MAX_RETRY_AFTER_SECS)


def worth_retrying(status: int, body: object) -> bool:
    if status == UNREACHABLE or status in RETRY_STATUSES:
        return True
    return status == 409 and isinstance(body, dict) and body.get("type") == IN_FLIGHT


def render_prompt(cfg: Config, text: str) -> str:
    """Fill a prompt's `{provider}`, `{harness}`, `{family}`, `{model}` and
    `{claimant_dir}` with this seat's own values. Nothing else in braces is
    touched, so a prompt can quote JSON."""
    # A zcode run has its own HOME, where `~` is not the runbook's home.
    claimant_dir = os.path.expanduser(cfg.claimant_dir) if cfg.harness == "zcode" else cfg.claimant_dir
    values = {"provider": cfg.provider, "harness": cfg.evidence_harness, "family": cfg.family,
              "model": cfg.model, "claimant_dir": claimant_dir}
    return PROMPT_FIELDS.sub(lambda m: values[m.group(1)], text)


def run_prompt(cfg: Config, repo: Repo, thread_id: str, lease_id: str, kind: str | None) -> str:
    prompt_path = cfg.prompts.get(kind or "", cfg.prompt)
    prompt = render_prompt(cfg, Path(prompt_path).read_text())
    prompt += (
        f"\n\nThis run's claim is already taken and acknowledged: thread_id "
        f"`{thread_id}`, claim_lease_id `{lease_id}`, repo `{repo.repo}`. Do not "
        "claim or acknowledge it again."
    )
    if prompt_path == cfg.prompt:
        prompt += (
            " Skip the runbook's sections \"0b. Before every claim: the toolchain\", "
            "\"1. Claim\" and \"2. Acknowledge\", and start at \"3. Read the task\"."
        )
    return prompt + "\n"


def needs_private_dir(cfg: Config) -> bool:
    """Gemini CLI and ZCode read MCP servers only from a file."""
    return cfg.harness in ("gemini", "zcode")


def write_private(path: Path, body: object) -> None:
    """Create `path` 0600, its missing parents 0700; never over a file that
    is already there."""
    # mkdir(parents=True) gives the intermediate directories the umask's mode.
    missing = []
    parent = path.parent
    while not parent.exists():
        missing.append(parent)
        parent = parent.parent
    for directory in reversed(missing):
        directory.mkdir(mode=0o700)
        os.chmod(directory, 0o700)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(body, f)


def harness_command(cfg: Config, repo: Repo, thread_id: str, lease_id: str,
                    kind: str | None = None, private: Path | None = None) -> tuple[list[str], dict[str, str]]:
    """The harness's argv and the variables it needs on top of `Runner.env`.
    The seat token is never in either: each harness reads it from
    `token_env` in its own environment."""
    prompt = run_prompt(cfg, repo, thread_id, lease_id, kind)
    if cfg.harness == "claude":
        return claude_command(cfg, repo, prompt), {}
    if cfg.harness in ("gemini", "zcode"):
        if private is None:
            raise ValueError(f"the {cfg.harness} harness needs a private directory for its config")
        if cfg.harness == "zcode":
            return zcode_command(cfg, repo, prompt, private)
        return gemini_command(cfg, repo, prompt, private)
    cmd = [
        *cfg.harness_bin, "exec",
        "-m", cfg.model,
        "--sandbox", "workspace-write",
        "--ephemeral",
        "-c", 'mcp_servers.maidan.default_tools_approval_mode="approve"',
    ]
    if repo.writable_roots:
        cmd += ["-c", "sandbox_workspace_write.writable_roots=" + json.dumps(repo.writable_roots)]
    return cmd + [prompt], {}


def claude_command(cfg: Config, repo: Repo, prompt: str) -> list[str]:
    """`claude -p`, with Maidan as the only MCP server, given inline so no
    file holds it. Its `headersHelper` prints the Authorization header from
    `token_env` when Claude Code connects. Bash runs in Claude Code's sandbox
    (writes to the checkout, the temp directory and each `--add-dir`; network
    to `allowed_domains`), with no retry outside it; edits are accepted and
    the Maidan tools approved, because nobody is there to answer a prompt."""
    helper = " ".join(shlex.quote(part) for part in (sys.executable, "-c", HEADERS_HELPER, cfg.token_env))
    mcp = {"mcpServers": {"maidan": {"type": "http", "url": cfg.mcp_url, "headersHelper": helper}}}
    settings = {"sandbox": {
        "enabled": True,
        "failIfUnavailable": True,
        "autoAllowBashIfSandboxed": True,
        "allowUnsandboxedCommands": False,
        "network": {"allowedDomains": cfg.allowed_domains},
    }}
    # The prompt comes straight after -p: --mcp-config, --allowedTools and
    # --add-dir each take several values and would swallow a trailing one.
    cmd = [
        *cfg.harness_bin, "-p", prompt,
        "--model", cfg.model,
        "--permission-mode", "acceptEdits",
        "--allowedTools", "mcp__maidan__*",
        "--strict-mcp-config",
        "--mcp-config", json.dumps(mcp),
        "--settings", json.dumps(settings),
        "--no-session-persistence",
    ]
    for root in repo.writable_roots:
        cmd += ["--add-dir", root]
    return cmd


def gemini_command(cfg: Config, repo: Repo, prompt: str, private: Path) -> tuple[list[str], dict[str, str]]:
    """`gemini -p` in its sandbox (on macOS, Seatbelt: writes to the checkout
    and each `--include-directories`), every tool approved, Maidan the only
    MCP server. Gemini takes MCP servers only from a settings file, so the
    runner writes one, 0600 in a 0700 directory, as the system settings file
    for this run. It names the token's variable, not the token: Gemini
    expands `${VAR}` in settings values."""
    settings = {"mcpServers": {"maidan": {
        "httpUrl": cfg.mcp_url,
        "headers": {"Authorization": "Bearer ${" + cfg.token_env + "}"},
        "timeout": 330_000,
        "trust": True,
    }}}
    path = private / "settings.json"
    write_private(path, settings)
    cmd = [
        *cfg.harness_bin, "-p", prompt,
        "-m", cfg.model,
        "--approval-mode", "yolo",
        "--sandbox",
        "--allowed-mcp-server-names", "maidan",
    ]
    if repo.writable_roots:
        cmd += ["--include-directories", ",".join(repo.writable_roots)]
    # Headless, nobody can answer the folder-trust check.
    return cmd, {"GEMINI_CLI_SYSTEM_SETTINGS_PATH": str(path), "GEMINI_CLI_TRUST_WORKSPACE": "true"}


def zcode_command(cfg: Config, repo: Repo, prompt: str, private: Path) -> tuple[list[str], dict[str, str]]:
    """`zcode --prompt` in guarded mode. ZCode has no flag for the model or
    the config: the model and its key go in a personal provider file
    (`ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`), and MCP servers only in
    `~/.zcode/cli/config.json`, read from HOME, so the run gets a private
    HOME. ZCode does not expand variables in either file, so the provider
    file holds the model key and the runtime config the seat token: each
    0600, under the run's 0700 directory, outside the checkout, and removed
    with it. Guarded mode runs ordinary tools without asking and fails the
    dangerous commands (force push, `git clean -f`, `rm -rf` and the like)
    that it would ask about, since nobody can answer; ZCode has no OS
    sandbox, so yolo, which would run them, is never used."""
    home = private / "home"
    provider_file = private / "provider_config.json"
    write_private(provider_file, {
        "schemaVersion": 1,
        "config": {
            "providerConfigRules": {"providerRules": [{
                "providerId": "seat",
                "providerName": cfg.provider,
                "config": {
                    "group": "standard-personal",
                    "access": {"type": "api-key", "apiKey": cfg.api_key},
                    "api": {"type": cfg.api_type, "baseUrl": cfg.api_base_url},
                    "personalModelIds": [cfg.model],
                },
            }]},
            "modelConfigRules": {"providerModelRules": [], "manualProviderModelRules": []},
            "defaultModelSelection": {"providerId": "seat", "modelId": cfg.model},
        },
    })
    write_private(home / ".zcode" / "cli" / "config.json", {
        "features": {"mcp": True},
        "mcp": {"servers": {"maidan": {
            "type": "http",
            "url": cfg.mcp_url,
            "headers": {"Authorization": f"Bearer {cfg.token}"},
            "timeoutMs": 330_000,
            "enabled": True,
        }}},
    })
    real_home = os.path.expanduser("~")
    env = {var: os.path.join(real_home, sub) for var, sub in HOME_PINNED.items() if var not in os.environ}
    env.update({
        "HOME": str(home),
        "ZCODE_DATA_BASE_DIR": str(home),
        "ZCODE_PERSONAL_PROVIDER_CONFIG_FILE": str(provider_file),
    })
    cmd = [
        *cfg.harness_bin,
        "--prompt", prompt,
        "--cwd", str(repo.checkout.resolve()),
        "--mode", "guarded",
        # Tools that wait on a person, who is not there.
        "--disallowed-tools", "AskUserQuestion EnterPlanMode ExitPlanMode",
        "--no-color",
    ]
    return cmd, env


RUN_DIR = re.compile(r"^run-(\d+)-")


def state_root(cfg: Config) -> Path:
    """The runner's 0700 directory for per-run files, refused if it is a
    symlink or someone else's."""
    root = cfg.state_dir
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    st = os.lstat(root)
    if not os.path.isdir(root) or os.path.islink(root) or st.st_uid != os.getuid():
        raise ConfigError(f"{root} is not a directory this user owns")
    os.chmod(root, 0o700)
    return root


def private_dir(cfg: Config) -> Path:
    """A fresh 0700 directory for one run, named for this runner's pid so a
    later runner can tell it was left by one that died."""
    return Path(tempfile.mkdtemp(prefix=f"run-{os.getpid()}-", dir=state_root(cfg)))


def sweep_private_dirs(cfg: Config) -> list[Path]:
    """Remove the run directories of runners that are no longer alive: a
    runner that was SIGKILLed or crashed could not remove its own."""
    if not cfg.state_dir.is_dir():
        return []
    removed = []
    for entry in state_root(cfg).iterdir():
        m = RUN_DIR.match(entry.name)
        if not m or int(m.group(1)) == os.getpid():
            continue
        try:
            os.kill(int(m.group(1)), 0)
            continue
        except ProcessLookupError:
            pass
        except PermissionError:
            continue
        shutil.rmtree(entry, ignore_errors=True)
        removed.append(entry)
    return removed


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
    """What one child has started. The child runs in its own session, so its
    process group is the first thing signalled. `capped` starts its command
    in another session, and once `capped` exits that command is reparented
    and no longer found under it, so the groups and pids seen while the
    child runs are kept, pruned to the live ones on every refresh.

    Nothing is signalled unless a fresh process table still ties it to this
    tree. Once the root is reaped its pid may be reused, so from then on the
    root pid is never walked from or signalled: only live members of the
    tracked groups, and tracked pids still in the group they were seen in,
    are reached. `lock` makes reaping and signalling exclusive, so no signal
    goes out between the reap and `root_reaped`."""

    def __init__(self, root: int) -> None:
        self.root = root
        self.reaped = False
        self.lock = threading.RLock()
        self.pids: dict[int, int] = {}  # pid -> pgid it was seen in
        self.groups: set[int] = {root}  # start_new_session: pgid == pid

    def root_reaped(self) -> None:
        with self.lock:
            self.reaped = True
            self.pids.pop(self.root, None)

    def _scan(self) -> tuple[dict[int, tuple[int, int]], dict[int, int]]:
        """The process table, and the pids in it that belong to this tree,
        each with its process group."""
        table = {pid: (ppid, pgid) for pid, ppid, pgid in process_table()}
        own = os.getpgrp()
        ours: dict[int, int] = {}
        if not self.reaped and self.root in table:
            children: dict[int, list[int]] = {}
            for pid, (ppid, _) in table.items():
                children.setdefault(ppid, []).append(pid)
            todo = [self.root]
            while todo:
                pid = todo.pop()
                if pid not in ours:
                    ours[pid] = table[pid][1]
                    todo.extend(children.get(pid, []))
        for pid, pgid in self.pids.items():
            if pid in table and table[pid][1] == pgid:
                ours[pid] = pgid
        for pid, (_, pgid) in table.items():
            if pgid in self.groups:
                ours[pid] = pgid
        if self.reaped:
            ours.pop(self.root, None)
        return table, {p: g for p, g in ours.items() if g != own}

    def refresh(self) -> dict[int, int]:
        with self.lock:
            table, ours = self._scan()
            self.pids = ours
            live = {pgid for _, pgid in table.values()}
            self.groups = {g for g in self.groups | set(ours.values()) if g in live and g != os.getpgrp()}
            if self.reaped and not any(g == self.root for g in ours.values()):
                self.groups.discard(self.root)
            return ours

    def alive(self) -> bool:
        return bool(self.refresh())

    def signal(self, sig: int) -> None:
        with self.lock:
            ours = self.refresh()
            for g in set(ours.values()):
                try:
                    os.killpg(g, sig)
                except (ProcessLookupError, PermissionError):
                    pass
            for p in ours:
                try:
                    os.kill(p, sig)
                except (ProcessLookupError, PermissionError):
                    pass

    def kill(self, grace: float) -> None:
        """SIGTERM, up to `grace` seconds, then SIGKILL whatever is left."""
        self.signal(signal.SIGTERM)
        deadline = time.monotonic() + grace
        while time.monotonic() < deadline and self.alive():
            time.sleep(0.2)
        if self.alive():
            self.signal(signal.SIGKILL)


class Runner:
    def __init__(self, cfg: Config, out=sys.stdout) -> None:
        self.cfg = cfg
        self.api = Maidan(cfg.url, cfg.token, cfg.timeout_secs)
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

    def harness_env(self) -> dict[str, str]:
        """The preflight's environment plus what the harness alone gets: the
        seat token, `[harness] env`, and each `env_from` variable."""
        env = {k: v for k, v in self.env(self.cfg.pass_token).items() if k not in self.cfg.harness_drop_env}
        return {**env, **self.cfg.harness_env, **self.cfg.harness_secrets}

    def host_problem(self, repo: Repo) -> str | None:
        load15 = os.getloadavg()[2]
        if load15 >= self.cfg.max_load15:
            return f"15-min load {load15:.1f} >= {self.cfg.max_load15}"
        free_gb = shutil.disk_usage(repo.checkout).free / 1024**3
        if free_gb < self.cfg.min_free_gb:
            return f"{free_gb:.0f} GB free < {self.cfg.min_free_gb}"
        return None

    def ready(self, repo: Repo) -> tuple[int | None, str, str]:
        """`(ready, outcome, detail)`: the count, or `None` with the outcome
        to log and why. An answer without a whole-number `ready` is not read
        as an empty queue: a server whose schema moved would look idle."""
        q = urllib.parse.urlencode({"engine": ENGINE, "repo": repo.repo})
        status, body = self.api.call("GET", f"/workspaces/{self.cfg.workspace_id}/queue-depth?{q}")
        if status == UNREACHABLE:
            return None, "queue_unreachable", str(body)
        if status != 200:
            return None, "queue_unreachable", f"queue-depth answered {status}"
        ready = body.get("ready") if isinstance(body, dict) else None
        if isinstance(ready, bool) or not isinstance(ready, int) or ready < 0:
            return None, "queue_unreadable", f"queue-depth answer without a whole-number ready: {str(body)[:200]}"
        return ready, "", ""

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
                with tree.lock:
                    code = proc.poll()
                    if code is not None:
                        tree.root_reaped()
                        break
                    tree.refresh()
                time.sleep(0.2)
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

    def write(self, path: str, body: object, attempts: int, backoff: float) -> tuple[int, object, int]:
        """POST with one Idempotency-Key across up to `attempts` tries,
        retrying no answer, 408, 429, 5xx and a key still in flight.
        Returns `(status, body, tries)`. The key is what makes a retry safe:
        a claim whose answer was lost is answered again, not taken again."""
        key = str(uuid.uuid4())
        status, answer = UNREACHABLE, None
        for attempt in range(attempts):
            status, answer, retry_after = self.api.send("POST", path, body, key=key)
            if not worth_retrying(status, answer):
                return status, answer, attempt + 1
            if attempt + 1 < attempts:
                # Not `stop.wait`: a claim the server may have taken, or a
                # release, is worth finishing even while the seat stops.
                time.sleep(retry_wait(attempt, backoff, retry_after))
        return status, answer, attempts

    def release(self, thread_id: str, lease_id: str) -> int:
        """Release, retrying with backoff. 2xx or 409 (the harness already
        released, or the lease lapsed) is final."""
        status, _, _ = self.write(f"/threads/{thread_id}/claim/release", {"claim_lease_id": lease_id},
                                  self.cfg.release_attempts, self.cfg.release_backoff_secs)
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
        ready, outcome, detail = self.ready(repo)
        if ready is None:
            return done(outcome, detail=detail)
        if ready == 0:
            return done("no_work", ready=0)
        problem = self.preflight(seat, repo)
        if problem:
            return done("preflight_failed", detail=problem)
        if self.stop.is_set():
            return done("stopped")
        status, claim, tries = self.write(
            f"/workspaces/{self.cfg.workspace_id}/threads/claim-next",
            {"engine": ENGINE, "repo": repo.repo, "lease_secs": self.cfg.lease_secs},
            self.cfg.claim_attempts, self.cfg.claim_backoff_secs,
        )
        if status != 200:
            detail = claim if status == UNREACHABLE else f"claim-next answered {status}"
            return done("claim_failed", status=status, tries=tries, detail=str(detail)[:300])
        if claim is None:
            return done("no_work", ready=ready)
        if not (isinstance(claim, dict) and isinstance(claim.get("id"), str)
                and isinstance(claim.get("claim_lease_id"), str)):
            # Nothing here to release with; the lease bounds how long it is held.
            return done("claim_failed", status=status, tries=tries,
                        detail=f"claim answer without id and claim_lease_id: {str(claim)[:200]}")
        thread_id, lease_id = claim["id"], claim["claim_lease_id"]
        attributes = claim.get("attributes")
        kind = attributes.get("kind") if isinstance(attributes, dict) else None
        outcome, code = "stopped", None
        stop_renew = threading.Event()
        renewer: threading.Thread | None = None
        try:
            ack, _, _ = self.write(f"/threads/{thread_id}/claim/acknowledge", {"claim_lease_id": lease_id},
                                   self.cfg.claim_attempts, self.cfg.claim_backoff_secs)
            if not 200 <= ack < 300:
                outcome = "ack_failed"
            elif not self.stop.is_set():
                def renew() -> None:
                    # A run longer than the lease would otherwise lose its
                    # claim mid-work. Wait before the first renewal so a short
                    # run is only claim, acknowledge, release. A lost lease
                    # stops quietly: the release that follows reports it.
                    while not stop_renew.wait(max(0.2, self.cfg.lease_secs / 3)) and not self.stop.is_set():
                        status, _, _ = self.write(
                            f"/threads/{thread_id}/claim/renew",
                            {"claim_lease_id": lease_id, "lease_secs": self.cfg.lease_secs},
                            self.cfg.claim_attempts, self.cfg.claim_backoff_secs,
                        )
                        if not 200 <= status < 300:
                            return

                renewer = threading.Thread(target=renew, daemon=True)
                renewer.start()
                private = private_dir(self.cfg) if needs_private_dir(self.cfg) else None
                try:
                    cmd, extra = harness_command(self.cfg, repo, thread_id, lease_id, kind, private)
                    code = self.run_process(
                        seat,
                        capped(self.cfg, self.cfg.run_secs, self.cfg.run_mem_gb, cmd),
                        repo.checkout,
                        {**self.harness_env(), **extra},
                    )
                finally:
                    # run_process returns only once the harness tree is dead,
                    # so nothing still reads the files.
                    if private is not None:
                        shutil.rmtree(private, ignore_errors=True)
                outcome = "stopped" if self.stop.is_set() else {
                    0: "ran", CAPPED_TIME: "timed_out", CAPPED_MEMORY: "out_of_memory",
                }.get(code, "failed")
        finally:
            stop_renew.set()
            if renewer is not None:
                renewer.join(2)
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
    try:
        sweep_private_dirs(cfg)
    except (ConfigError, OSError) as e:
        print(f"seat-runner: {e}", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, runner.shutdown)
    signal.signal(signal.SIGINT, runner.shutdown)
    return runner.run(args.once)


if __name__ == "__main__":
    sys.exit(main())
