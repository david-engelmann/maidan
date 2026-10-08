"""Answer the pending approval gates from a terminal: the twin of `/ui`.

    MAIDAN_APPROVAL_TOKEN=maid_… python approve.py   # accept every pending gate
    python approve.py --decline                       # decline them, as the admin

Accepting a gate needs a token holding `approval:grant`, or a browser session
a person signed in to through the identity provider. The admin token
`maidan init` printed does not hold it, and a plain token cannot accept: it
is exactly the token a person hands an agent. An admin mints the approver
token on purpose (the command is printed below when it is missing).
Declining needs only the admin token.

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
    url = os.environ.get("MAIDAN_URL", "http://maidan:8080")
    workspace_id, token = admin_from_init()
    admin = Maidan(url, token)
    answerer = admin
    if action == "accept":
        approver = os.environ.get("MAIDAN_APPROVAL_TOKEN", "").strip()
        if not approver:
            me = admin.call("GET", "/me")
            print(
                "approve: accepting needs a token holding approval:grant (or accept in /ui,\n"
                "signed in through your identity provider). Mint one as the admin, then\n"
                "pass it as MAIDAN_APPROVAL_TOKEN:\n\n"
                f"  curl -X POST {url}/workspaces/{workspace_id}/members/{me['member_id']}/tokens \\\n"
                "    -H 'Authorization: Bearer <admin token>' -H 'Content-Type: application/json' \\\n"
                "    -d '{\"label\":\"approver\",\"capabilities\":"
                "[\"workspace:read\",\"workspace:write\",\"approval:grant\"]}'",
                file=sys.stderr,
            )
            return 2
        answerer = Maidan(url, approver)
    pending = admin.call("GET", f"/workspaces/{workspace_id}/approval-gates")
    if not pending:
        print("approve: no pending gates")
        return 1
    for view in pending:
        gate = view.get("gate", view)
        answerer.call(
            "POST",
            f"/approval-gates/{gate['id']}/answer",
            {"action": action, "request_state": view["request_state"]},
        )
        print(f"approve: {action}: {gate['prompt']} ({gate['id']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
