#!/usr/bin/env python3
"""Path gates for backup.sh / restore.sh.

Validates backup output dirs, restore sources, and ARTIFACT_LOCALFS_ROOT so a
typo with ``..`` or a bare system root cannot wipe or unpack into the wrong
place. Invoked as::

    python3 scripts/backup_path_gates.py backup-dir PATH
    python3 scripts/backup_path_gates.py restore-src PATH
    python3 scripts/backup_path_gates.py artifact-root PATH
"""

from __future__ import annotations

import sys
from pathlib import PurePosixPath

_FORBIDDEN_ROOTS = frozenset(
    {
        "/",
        "/tmp",
        "/var",
        "/etc",
        "/home",
        "/Users",
        "/root",
        "/dev",
        "/proc",
        "/sys",
    }
)


def _refuse_controls(raw: str) -> str:
    if not isinstance(raw, str):
        raise ValueError("path must be a string")
    if not raw or raw.strip() != raw:
        raise ValueError(f"path must be non-empty and trimmed: {raw!r}")
    for ch in raw:
        if ord(ch) < 32:
            raise ValueError("path must not contain control characters")
    return raw


def _strip_trailing_slashes(path: str) -> str:
    while len(path) > 1 and path.endswith("/"):
        path = path[:-1]
    return path


def _parts_ok(path: str) -> None:
    normalized = path.replace("\\", "/")
    if len(normalized) >= 2 and normalized[1] == ":":
        raise ValueError(f"path must not be a Windows drive path: {path!r}")
    pure = PurePosixPath(normalized)
    if ".." in pure.parts:
        raise ValueError(f"path must not contain '..': {path!r}")
    if "//" in normalized:
        raise ValueError(f"path must not contain empty segments: {path!r}")


def require_backup_dir(path: str) -> str:
    """Backup output directory (relative or absolute)."""
    path = _strip_trailing_slashes(_refuse_controls(path))
    _parts_ok(path)
    if path in _FORBIDDEN_ROOTS or path in {".", ""}:
        raise ValueError(f"backup dir must not be a system root or '.': {path!r}")
    return path


def require_restore_src(path: str) -> str:
    """Backup directory passed to restore.sh."""
    return require_backup_dir(path)


def require_artifact_root(path: str) -> str:
    """ARTIFACT_LOCALFS_ROOT for tar/untar."""
    path = _strip_trailing_slashes(_refuse_controls(path))
    _parts_ok(path)
    if not path.startswith("/"):
        raise ValueError(f"artifact root must be absolute: {path!r}")
    if path in _FORBIDDEN_ROOTS:
        raise ValueError(f"artifact root must not be a system root: {path!r}")
    pure = PurePosixPath(path)
    if len(pure.parts) < 3:
        raise ValueError(f"artifact root is too shallow: {path!r}")
    return path


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in {"backup-dir", "restore-src", "artifact-root"}:
        print(
            "usage: backup_path_gates.py backup-dir|restore-src|artifact-root <path>",
            file=sys.stderr,
        )
        return 2
    try:
        if argv[1] == "backup-dir":
            print(require_backup_dir(argv[2]))
        elif argv[1] == "restore-src":
            print(require_restore_src(argv[2]))
        else:
            print(require_artifact_root(argv[2]))
    except ValueError as exc:
        print(f"backup-path-gates: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
