"""CLI-driven providers: Grok API, Gemini CLI, Copilot CLI, Claude Code, Cursor.

Unlike browser providers, CLI providers need no browser, auth state machine,
or selectors. They need: env keys or binaries (environment, not code),
idempotent install(), a fast is_setup() probe, and run_prompt() over a
subprocess. Server-side assertions reuse harness.probe.ServerProbe unchanged.

Elicitation rule (hard): only Claude Code supports elicitation. No other
provider may attempt it; tests/enable_elicitation.py guards this.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys

from .. import (
    KEY_MISSING, BINARY_MISSING, CONFIG_ERROR,
    EXIT_KEY, EXIT_BINARY, EXIT_CONFIG,
)


class EnvError(Exception):
    """A required env var is absent. Fail fast; never prompt for secrets."""
    def __init__(self, var: str):
        self.var = var
        super().__init__(f"{KEY_MISSING} {var} is not set; export it and retry "
                         f"(never hardcode keys)")


class ConfigError(Exception):
    """A server/config problem that is NOT a missing key (e.g. the MCP URL
    is loopback and the provider's API cannot reach it). Carries its own
    signal so triage never misreads it as KEY_MISSING."""
    def __init__(self, detail: str):
        self.detail = detail
        super().__init__(f"{CONFIG_ERROR} {detail}")


class BinaryError(Exception):
    """A required CLI binary is absent and auto-install failed."""
    def __init__(self, binary: str, hint: str):
        self.binary = binary
        super().__init__(f"{BINARY_MISSING} {binary} not found and auto-install "
                         f"failed; {hint}")


def die_env(e: EnvError) -> "None":
    print(str(e), file=sys.stderr)
    sys.exit(EXIT_KEY)


def die_config(e: ConfigError) -> "None":
    print(str(e), file=sys.stderr)
    sys.exit(EXIT_CONFIG)


def die_binary(e: BinaryError) -> "None":
    print(str(e), file=sys.stderr)
    sys.exit(EXIT_BINARY)


class CLIProvider:
    """Base for subprocess-driven providers.

    Subclasses set: name, required_env, binary, npm_package, and implement
    install()/is_setup()/run_prompt(). manual_only marks providers that
    cannot run unattended (cursor, meta) — the CLI refuses test/nightly
    for them with MANUAL_ONLY.
    """
    name = "cli-base"
    required_env: tuple[str, ...] = ()
    # At least one of these must be set (providers with several token names).
    required_env_any: tuple[str, ...] = ()
    binary: str | None = None
    npm_package: str | None = None
    manual_only: bool = False
    # checklist_only: no setup/install path exists at all (meta). setup and
    # setup-verify refuse with MANUAL_ONLY pointing at the checklist, instead
    # of crashing on an unimplemented install().
    checklist_only: bool = False
    # Checklist filename override; defaults to checklists/<name>.md.
    checklist_file: str | None = None
    # Elicitation: True ONLY for Claude Code. The guard test
    # (tests/test_harness.py::test_elicitation_only_claude_code) fails
    # if any other provider sets this.
    supports_elicitation: bool = False

    def __init__(self, cfg: dict):
        self.cfg = cfg
        self.pcfg = cfg["providers"].get(self.name, {})

    @property
    def checklist_path(self) -> str:
        return self.checklist_file or f"checklists/{self.name}.md"

    # -- environment ----------------------------------------------------
    def check_env(self) -> None:
        """Raise EnvError on the first missing required env var.

        required_env: every var must be set. required_env_any: at least one
        must be set (for providers that accept several token names).
        """
        for var in self.required_env:
            if not os.environ.get(var):
                raise EnvError(var)
        if self.required_env_any and not any(
                os.environ.get(v) for v in self.required_env_any):
            raise EnvError("/".join(self.required_env_any))

    def ensure_binary(self) -> str:
        """Return the binary path, auto-installing via npm if configured.

        Raises BinaryError when absent and auto-install failed or unconfigured.
        """
        if not self.binary:
            return ""
        path = shutil.which(self.binary)
        if path:
            return path
        if self.npm_package:
            print(f"installing {self.npm_package} via npm...", file=sys.stderr)
            r = subprocess.run(
                ["npm", "install", "-g", self.npm_package],
                capture_output=True, text=True, timeout=600)
            if r.returncode == 0:
                path = shutil.which(self.binary)
                if path:
                    print(f"installed {self.binary} -> {path}", file=sys.stderr)
                    return path
        raise BinaryError(
            self.binary,
            f"install it manually"
            + (f" (`npm install -g {self.npm_package}`)" if self.npm_package else "")
            + " and retry")

    def run(self, argv: list[str], timeout: int = 120,
            env: dict | None = None) -> subprocess.CompletedProcess:
        """Run a subprocess; never prompts, never hangs past timeout.

        stdin is DEVNULL: a CLI that tries to prompt (e.g. an interactive
        login) fails fast on EOF instead of blocking on a human until the
        timeout expires.
        """
        return subprocess.run(argv, capture_output=True, text=True,
                              timeout=timeout, env=env, stdin=subprocess.DEVNULL)

    # -- lifecycle (subclass implements) ---------------------------------
    def install(self) -> None:
        """Idempotent install of the MCP/plugin wiring. Assumes env+binary ok."""
        raise NotImplementedError

    def is_setup(self) -> bool:
        """Fast probe: is the wiring present and pointed at the dev server?"""
        raise NotImplementedError

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        """Run one prompt end-to-end; return the assistant's text."""
        raise NotImplementedError
