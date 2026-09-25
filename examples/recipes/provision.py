"""Install one recipe agent as an app, with a token scoped to what it does.

Runs once, as the admin `maidan init` created. The admin token stays on the
server's data volume; the agent's container only ever sees its own narrow
token, written to the `agent_creds` volume. A rerun finds that file and exits.

    python provision.py coder|deployer
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import sys

from maidan_http import Maidan, MaidanError

INIT_OUTPUT = pathlib.Path("/data/recipe/init.txt")
CREDS = pathlib.Path("/creds/agent.json")

ROLES = {
    "coder": {
        "channel": "coding",
        "capabilities": [
            "workspace:read",
            "message:post",
            "thread:transition",
            "artifact:upload",
        ],
        "title": "Add a CONTRIBUTORS file",
        "task": "Add a CONTRIBUTORS file listing the maintainers, one per line.",
    },
    "deployer": {
        "channel": "deploys",
        "capabilities": ["workspace:read", "message:post", "thread:transition"],
        "title": "Deploy v9 to production",
        "task": "Roll out v9 to production.",
    },
}


def admin_from_init() -> tuple[str, str]:
    text = INIT_OUTPUT.read_text()
    workspace = re.search(r"workspace:\s+\S+\s+\(([0-9a-f-]{36})\)", text)
    token = re.search(r"^\s+(maid_\S+)\s*$", text, re.MULTILINE)
    if not workspace or not token:
        sys.exit(f"provision: cannot read the admin token from {INIT_OUTPUT}")
    return workspace.group(1), token.group(1)


def find_or_create(admin: Maidan, list_path: str, key: str, value: str, body: dict) -> dict:
    """Idempotent create, so a provision that died halfway can be rerun."""
    for row in admin.call("GET", list_path):
        if row.get(key) == value:
            return row
    return admin.call("POST", list_path, body)


def main() -> int:
    role = sys.argv[1] if len(sys.argv) > 1 else ""
    if role not in ROLES:
        sys.exit(f"usage: provision.py {'|'.join(ROLES)}")
    if CREDS.exists():
        print(f"provision: {role} already provisioned")
        return 0
    spec = ROLES[role]
    url = os.environ.get("MAIDAN_URL", "http://maidan:8080")
    workspace_id, admin_token = admin_from_init()
    admin = Maidan(url, admin_token)

    channel = find_or_create(
        admin,
        f"/workspaces/{workspace_id}/channels",
        "name",
        spec["channel"],
        {"name": spec["channel"], "private": False},
    )
    # An agent joins a workspace as an installed app: the installation creates
    # its member (`app:<slug>`) and caps what any of its tokens may hold.
    app = find_or_create(
        admin,
        f"/workspaces/{workspace_id}/apps",
        "slug",
        role,
        {"slug": role, "name": f"Recipe {role}"},
    )
    installation = admin.call(
        "POST",
        f"/workspaces/{workspace_id}/apps/{app['id']}/install",
        {"granted_capabilities": spec["capabilities"]},
    )
    minted = admin.call(
        "POST",
        f"/workspaces/{workspace_id}/app-installations/{installation['id']}/tokens",
        {"label": f"recipe-{role}", "capabilities": spec["capabilities"]},
    )
    member_id = minted["bot_member_id"]

    # The task is filed by the admin, standing in for whoever assigns the work.
    try:
        thread = admin.call(
            "POST", f"/channels/{channel['id']}/threads", {"title": spec["title"]}
        )
        admin.call(
            "POST",
            f"/threads/{thread['id']}/messages",
            {"body": os.environ.get("RECIPE_TASK") or spec["task"]},
        )
    except MaidanError as error:
        sys.exit(f"provision: could not file the task: {error}")

    CREDS.write_text(
        json.dumps(
            {
                "url": url,
                "token": minted["secret"],
                "workspace_id": workspace_id,
                "channel_id": channel["id"],
                "member_id": member_id,
            }
        )
    )
    CREDS.chmod(0o600)
    print(
        f"provision: {role} (app:{role}, {member_id}) may {', '.join(spec['capabilities'])}; "
        f"filed '{spec['title']}' in #{spec['channel']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
