#!/usr/bin/env python3
"""Pure path gates for book/sync-docs.sh.

Validates curated page and asset names before they are copied into
``book/src/docs/``, so a typo with ``..`` or a path separator cannot stage
outside the generated tree. Invoked as::

    python3 book/sync-docs-gates.py check-name "Capability Map"
    python3 book/sync-docs-gates.py check-asset "maidan-mark.svg"
"""

from __future__ import annotations

import sys


def _refuse_traversal(label: str, name: str) -> str:
    if not isinstance(name, str):
        raise ValueError(f"{label} must be a string")
    if not name or name.strip() != name:
        raise ValueError(f"{label} must be non-empty and trimmed: {name!r}")
    for bad in ("..", "/", "\\", "\0", "\n", "\r"):
        if bad in name:
            raise ValueError(f"{label} must not contain {bad!r}: {name!r}")
    return name


def validate_page_name(name: str) -> str:
    name = _refuse_traversal("page name", name)
    if name.startswith("-") or name.endswith("-"):
        raise ValueError(f"page name must not start or end with a hyphen: {name!r}")
    return name


def validate_asset_name(name: str) -> str:
    name = _refuse_traversal("asset name", name)
    if name.startswith("."):
        raise ValueError(f"asset name must be a plain file name: {name!r}")
    return name


def staged_markdown_name(page_name: str) -> str:
    """Space-named docs pages are staged under hyphenated names."""
    return validate_page_name(page_name).replace(" ", "-") + ".md"


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in {"check-name", "check-asset", "stage-name"}:
        print(
            "usage: sync-docs-gates.py check-name|check-asset|stage-name <value>",
            file=sys.stderr,
        )
        return 2
    try:
        if argv[1] == "check-name":
            print(validate_page_name(argv[2]))
        elif argv[1] == "check-asset":
            print(validate_asset_name(argv[2]))
        else:
            print(staged_markdown_name(argv[2]))
    except ValueError as exc:
        print(f"sync-docs-gates: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
