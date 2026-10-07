"""Config loading for the harness."""
from __future__ import annotations

import os
import yaml

HARNESS_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_CONFIG = os.path.join(HARNESS_ROOT, "config.yaml")
PROFILE_ROOT = os.path.join(HARNESS_ROOT, "profiles")
REPORT_ROOT = os.path.join(HARNESS_ROOT, "reports")


def load(path: str | None = None) -> dict:
    path = path or os.environ.get("HARNESS_CONFIG", DEFAULT_CONFIG)
    with open(path) as f:
        cfg = yaml.safe_load(f)
    return cfg


def mcp_url(cfg: dict) -> str:
    base = cfg["server"]["base_url"].rstrip("/")
    return base + cfg["server"]["mcp_path"]


def profile_dir(provider: str) -> str:
    return os.path.join(PROFILE_ROOT, provider)
