#!/usr/bin/env python3
"""An OpenHands agent that works one Maidan task per run.

It connects to Maidan over MCP (streamable HTTP) with its own agent token,
claims the next task on a channel, does it with OpenHands' terminal and file
editor, and hands the result back for a person to review. Maidan runs the
waiter loop around the work (claim, acknowledge, renew, result, review,
release); the task text is whatever a person or another agent filed.

    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...      # an agent token, not the admin token
    python maidan_claimant.py --check                  # connect and list tools; no model
    export LLM_MODEL=anthropic/claude-sonnet-4-5-20250929 LLM_API_KEY=...
    python maidan_claimant.py --channel <channel uuid>  # one task, then exit

Needs openhands-sdk and openhands-tools (see requirements.txt). README.md has
the token setup.
"""

import argparse
import os
import sys
from urllib.parse import urlsplit

from openhands.sdk.mcp import create_mcp_tools
from openhands.sdk.mcp.config import coerce_mcp_config

# The ten Maidan tools the loop uses. OpenHands sends every tool it has on
# every model request (there is no tool search), so the agent sees only these
# and its own two, not Maidan's whole catalog.
LOOP_TOOLS = (
    "claim_next_thread",
    "acknowledge_claim",
    "get_thread_context",
    "post_message",
    "renew_claim",
    "request_approval",
    "get_approval_gate",
    "set_thread_result",
    "transition_thread",
    "release_claim",
)

INSTRUCTIONS = """\
You are one worker in a Maidan workspace. Other agents and people file tasks
as threads on a channel; you take one, do it, and hand it back. Follow these
steps exactly, using the Maidan tools.

1. Call claim_next_thread with channel_id "{channel}" and lease_secs
   {lease}. If it returns null there is no work: say "no work" and stop. Do
   not claim anything else.
2. Keep the thread's id and claim_lease_id. Call acknowledge_claim with both.
3. Call get_thread_context with the thread_id. The first message is the task;
   later messages are discussion. Do what the task asks, with the terminal
   and file editor, in the current directory.
4. Post short progress notes with post_message (thread_id, body). If the work
   will take longer than {lease} seconds, call renew_claim (thread_id,
   claim_lease_id, lease_secs {lease}) before the lease runs out.
5. If the task needs a person's decision before you act (deploying, deleting,
   spending money), call request_approval with a clear prompt and the
   thread_id, keep the gate_id it returns, then poll get_approval_gate
   (gate_id) until its state is not "pending".
   Go ahead only if it is "accepted". Never approve your own request.
6. When you are done, call set_thread_result with thread_id and a result
   object: {{"status": "done" or "failed", "summary": "<what you did>"}}.
7. Call transition_thread with the thread_id and action "start_review". This
   is what keeps the task from being handed out again. Never "close" it:
   someone else reviews and closes.
8. Always finish with release_claim (thread_id, claim_lease_id), even if you
   failed. If you failed, record status "failed" in step 6 first.
"""


def maidan_url() -> str:
    base = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080").rstrip("/")
    parts = urlsplit(base)
    # The token rides on every request, so plain http is for this machine only.
    if parts.scheme == "http" and parts.hostname not in ("127.0.0.1", "localhost", "::1"):
        sys.exit(f"refusing to send the token to {base} over plain http; use https or a loopback address")
    if parts.scheme not in ("http", "https") or parts.username or parts.password:
        sys.exit(f"MAIDAN_URL must be a plain http(s) URL, got {base}")
    return base


def mcp_config(base: str, token: str):
    return coerce_mcp_config(
        {
            "maidan": {
                "url": f"{base}/mcp/streamable",
                "transport": "http",
                "headers": {"Authorization": f"Bearer {token}"},
                # A claim can wait on a person's approval; keep calls alive.
                "timeout": 300,
            }
        }
    )


def check(config) -> int:
    """Connect, list Maidan's tools and confirm the loop's ten are there."""
    with create_mcp_tools(config, timeout=60) as client:
        names = {tool.name for tool in client.tools}
    missing = [name for name in LOOP_TOOLS if name not in names]
    print(f"connected: {len(names)} Maidan tools offered to this token")
    if missing:
        print("missing for the loop (check the token's capabilities): " + ", ".join(missing))
        return 1
    print("the loop's ten tools are all there")
    return 0


def work(config, channel: str, lease: int) -> int:
    from pydantic import SecretStr

    from openhands.sdk import LLM, Agent, Conversation, Tool
    from openhands.tools.file_editor import FileEditorTool
    from openhands.tools.terminal import TerminalTool

    api_key = os.environ.get("LLM_API_KEY")
    model = os.environ.get("LLM_MODEL")
    if not api_key or not model:
        sys.exit("set LLM_MODEL (LiteLLM form, e.g. anthropic/claude-sonnet-4-5-20250929) and LLM_API_KEY")
    llm = LLM(
        usage_id="maidan-claimant",
        model=model,
        base_url=os.environ.get("LLM_BASE_URL"),
        api_key=SecretStr(api_key),
    )
    keep = "|".join((TerminalTool.name, FileEditorTool.name) + LOOP_TOOLS)
    agent = Agent(
        llm=llm,
        tools=[Tool(name=TerminalTool.name), Tool(name=FileEditorTool.name)],
        mcp_config=config,
        filter_tools_regex=f"^({keep})$",
    )
    conversation = Conversation(agent=agent, workspace=os.getcwd())
    conversation.send_message(INSTRUCTIONS.format(channel=channel, lease=lease))
    conversation.run()
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--channel", help="the Maidan channel to take work from (a UUID)")
    parser.add_argument("--lease-secs", type=int, default=600, help="how long a claim lasts (default 600)")
    parser.add_argument("--check", action="store_true", help="connect and list tools, then exit; no model is called")
    args = parser.parse_args()

    token = os.environ.get("MAIDAN_TOKEN")
    if not token:
        sys.exit("set MAIDAN_TOKEN to the agent's token")
    config = mcp_config(maidan_url(), token)
    if args.check:
        return check(config)
    if not args.channel:
        parser.error("--channel is required unless --check")
    return work(config, args.channel, args.lease_secs)


if __name__ == "__main__":
    sys.exit(main())
