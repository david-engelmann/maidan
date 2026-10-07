"""Timestamped run reports. A reviewer verifies in 10 minutes without
reading Playwright code: the report leads with commands run, tool calls
observed server-side, and pass/fail per suite.
"""
from __future__ import annotations

import datetime
import json
import os

from . import config as config_mod


class Report:
    def __init__(self, cfg: dict, title: str):
        self.cfg = cfg
        self.title = title
        self.started = datetime.datetime.now(datetime.timezone.utc)
        self.events: list[dict] = []
        self.header = {
            "title": title,
            "started_utc": self.started.isoformat(),
            "server_base_url": cfg["server"]["base_url"],
            "server_mcp_url": config_mod.mcp_url(cfg),
            "server_commit_sha": cfg["server"].get("commit_sha") or "UNRECORDED",
        }

    def event(self, kind: str, **fields) -> None:
        self.events.append({"kind": kind, **fields})

    def command(self, argv: list[str]) -> None:
        self.event("command", argv=argv)

    def tool_called(self, provider: str, tool: str, suite: str,
                    correlation: str = "unknown") -> None:
        self.event("tool_called", provider=provider, tool=tool, suite=suite,
                   correlation=correlation)

    def suite_result(self, provider: str, suite: str, ok: bool, detail: str = "") -> None:
        self.event("suite_result", provider=provider, suite=suite,
                   ok=ok, detail=detail)

    def failure(self, signal: str, detail: str = "") -> None:
        self.event("failure", signal=signal, detail=detail)

    def write(self) -> str:
        os.makedirs(config_mod.REPORT_ROOT, exist_ok=True)
        stamp = self.started.strftime("%Y%m%d-%H%M%S")
        path = os.path.join(config_mod.REPORT_ROOT, f"{stamp}-{self.title}.json")
        ended = datetime.datetime.now(datetime.timezone.utc)
        doc = {
            **self.header,
            "ended_utc": ended.isoformat(),
            "duration_s": round((ended - self.started).total_seconds(), 1),
            "events": self.events,
        }
        with open(path, "w") as f:
            json.dump(doc, f, indent=2)
        return path

    def summary_lines(self) -> list[str]:
        lines = [
            f"report: {self.title}",
            f"server: {self.header['server_base_url']} @ {self.header['server_commit_sha']}",
        ]
        for e in self.events:
            k = e["kind"]
            if k == "suite_result":
                mark = "PASS" if e["ok"] else "FAIL"
                lines.append(f"[{mark}] {e['provider']}/{e['suite']} {e.get('detail','')}".rstrip())
            elif k == "tool_called":
                lines.append(f"  tool observed server-side: {e['tool']} ({e['provider']}/{e['suite']}, via {e.get('correlation','?')})")
            elif k == "failure":
                lines.append(f"  FAILURE {e['signal']} {e.get('detail','')}".rstrip())
            elif k == "suite_skipped":
                lines.append(f"  [SKIP] {e['provider']}/{e['suite']}: {e.get('reason','')}")
            elif k == "auth_verdict":
                lines.append(f"  auth {e['provider']}: {e.get('verdict')} seen={e.get('seen')} missing={e.get('missing')}")
            elif k == "setup_probe":
                lines.append(f"  setup {e['provider']}: ok={e.get('ok')} {e.get('detail','')}")
        return lines
