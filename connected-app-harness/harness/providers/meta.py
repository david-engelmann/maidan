"""Meta Muse: manual-only. No code paths.

Research §1.7: muse.ai/platform is a sign-in-gated form; no connector SDK,
no protocol spec, no testing API is published as of 2026-10-06. Multiple
sources confirm there is NO API for driving the Muse assistant itself.
The working path is a conversational custom connector (Meta Help Center:
"ask Muse to create a Custom Connector"), credentials in Meta's Secure
Credentials Store. Walkthrough: checklists/meta-muse.md.

This provider exists so the CLI can name it: `harness test meta` refuses
with MANUAL_ONLY instead of "unknown provider".
"""
from __future__ import annotations

from .cli_base import CLIProvider


class MetaProvider(CLIProvider):
    surface = "manual"
    name = "meta"
    manual_only = True
    checklist_only = True
    checklist_file = "checklists/meta-muse.md"
    binary = None

    def install(self) -> None:
        raise NotImplementedError(
            "meta has no install API; follow checklists/meta-muse.md")

    def is_setup(self) -> bool:
        # Cannot be probed programmatically.
        return False

    def run_prompt(self, prompt: str, timeout: int = 180) -> str:
        raise NotImplementedError(
            "meta has no automation API (research §1.7); manual walkthrough only.")
