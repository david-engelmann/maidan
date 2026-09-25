"""Answer the pending approval gates, as the admin: the terminal twin of `/ui`.

    python approve.py            # accept every pending gate
    python approve.py --decline  # decline them

Each answer echoes the `request_state` the server signed when it listed the
gate. The agent that asked cannot accept its own request, whatever token it holds.
"""

from __future__ import annotations

import os
import sys

from maidan_http import Maidan
from provision import admin_from_init


def main() -> int:
    action = "decline" if "--decline" in sys.argv else "accept"
    workspace_id, token = admin_from_init()
    admin = Maidan(os.environ.get("MAIDAN_URL", "http://maidan:8080"), token)
    pending = admin.call("GET", f"/workspaces/{workspace_id}/approval-gates")
    if not pending:
        print("approve: no pending gates")
        return 1
    for view in pending:
        gate = view.get("gate", view)
        admin.call(
            "POST",
            f"/approval-gates/{gate['id']}/answer",
            {"action": action, "request_state": view["request_state"]},
        )
        print(f"approve: {action}: {gate['prompt']} ({gate['id']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
