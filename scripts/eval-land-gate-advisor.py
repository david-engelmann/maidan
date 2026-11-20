#!/usr/bin/env python3
"""Measure the land-gate advisor against labelled hand-offs.

For each case in the labelled set this makes a task on a running Maidan,
claims it, posts the agent's messages, sets the result, and asks for advice
(`advise_land_gate`). It then reports, for every judgment and for the advice,
how many answers agree with the labels. Nothing is approved or closed, and the
land gate is never written.

    eval-land-gate-advisor.py --maidan https://maidan.example.com --token-env MAIDAN_TOKEN

The token needs to create a channel and tasks, claim, and ask for advice (an
admin's does). The instance must have advice on and a TypeSafe key for the
token's workspace. Every hand-off in the set is sent to the provider.

Exit status: 0 when no labelled judgment was answered wrong and the advice
agreed at least `--min-land` of the time; 1 otherwise; 2 on a setup failure.

Standard library only.
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--maidan", required=True, help="the instance's origin")
    parser.add_argument("--token-env", default="MAIDAN_TOKEN", help="the environment variable holding the bearer token")
    parser.add_argument("--cases", type=Path, default=HERE / "land-gate-advice-cases.json")
    parser.add_argument("--only", help="comma-separated case ids")
    parser.add_argument("--min-land", type=float, default=0.9, help="the share of cases whose advice must match its label")
    parser.add_argument("--out", type=Path, help="write every answer here as JSON lines")
    args = parser.parse_args()
    token = os.environ.get(args.token_env, "").strip()
    if not token:
        print(f"{args.token_env} is not set", file=sys.stderr)
        return 2
    origin = args.maidan.rstrip("/")
    cases = json.loads(args.cases.read_text(encoding="utf-8"))["cases"]
    if args.only:
        wanted = set(args.only.split(","))
        cases = [c for c in cases if c["id"] in wanted]

    calls = 0

    def tool(name: str, arguments: dict):
        nonlocal calls
        calls += 1
        body = json.dumps({"jsonrpc": "2.0", "id": calls, "method": "tools/call", "params": {"name": name, "arguments": arguments}}).encode()
        request = urllib.request.Request(
            f"{origin}/mcp/streamable",
            data=body,
            method="POST",
            # Named, because a hosted instance's edge refuses Python's default client.
            headers={"authorization": f"Bearer {token}", "content-type": "application/json", "accept": "application/json, text/event-stream", "user-agent": "maidan-advice-eval/1"},
        )
        for attempt in range(5):
            try:
                with urllib.request.urlopen(request, timeout=60) as response:
                    raw = response.read().decode()
            except urllib.error.HTTPError as error:
                raise SystemExit(f"{name}: HTTP {error.code}: {error.read().decode()[:300]}")
            # A streamable answer may come as one server-sent event.
            if raw.lstrip().startswith("event:") or raw.lstrip().startswith("data:"):
                raw = next((line[5:].strip() for line in raw.splitlines() if line.startswith("data:")), "{}")
            answer = json.loads(raw)
            message = answer.get("error", {}).get("message", "")
            if message.startswith("rate limited") and attempt < 4:
                time.sleep(2 + attempt * 2)
                continue
            if "error" in answer:
                raise SystemExit(f"{name}: {message}")
            text = "".join(part.get("text", "") for part in answer["result"].get("content", []))
            if answer["result"].get("isError"):
                raise SystemExit(f"{name}: {text[:300]}")
            return json.loads(text) if text else None
        raise SystemExit(f"{name}: still rate limited")

    stamp = time.strftime("%Y%m%d%H%M%S", time.gmtime())
    workspace = tool("whoami", {})["workspace_id"]
    channel = tool("create_channel", {"workspace_id": workspace, "name": f"advice-eval-{stamp}"})["id"]
    judged = {}  # judgment -> [agree, unsettled, wrong]
    wrong_lines = []
    land_agree = 0
    tokens = 0
    sink = args.out.open("w", encoding="utf-8") if args.out else None
    print(f"{'case':26} {'advice':7} {'label':7}  why")
    for case in cases:
        filed = {"channel_id": channel, "title": case["task"]["title"]}
        if case["task"].get("description"):
            filed["description"] = case["task"]["description"]
        thread = tool("create_thread", filed)["id"]
        # The ask, as a dispatcher posts it. The advisor leaves a thread's
        # opening message out when its author made the thread, so without
        # this the agent's first message below would be taken for it.
        tool("post_message", {"thread_id": thread, "body": case["task"].get("description") or case["task"]["title"]})
        claim = tool("claim_thread", {"thread_id": thread})
        claimed = claim.get("thread", claim)
        for body in case.get("agent_messages", []):
            tool("post_message", {"thread_id": thread, "body": body})
        tool("set_thread_result", {"thread_id": thread, "result": case["result"], "claim_lease_id": claimed["claim_lease_id"]})
        advice = tool("advise_land_gate", {"thread_id": thread, "refresh": True})
        # Give the task back: a claim held for every case would run into a
        # workspace's limit on work in progress.
        tool("release_claim", {"thread_id": thread, "claim_lease_id": claimed["claim_lease_id"]})
        tokens += advice["usage"]["input_tokens"]
        if sink:
            sink.write(json.dumps({"id": case["id"], "advice": advice}) + "\n")
        expect = case["expect"]
        agrees = advice["land"] == expect["land"]
        land_agree += agrees
        print(f"{case['id'][:25]:26} {advice['land']:7} {expect['land']:7}{'' if agrees else ' !'} {' '.join(advice['reasons'])}")
        for judgment in advice["judgments"]:
            label = expect["judgments"].get(judgment["id"])
            if label is None:
                continue
            row = judged.setdefault(judgment["id"], [0, 0, 0])
            if judgment["answer"] == "unsettled":
                row[1] += 1
            elif (judgment["answer"] == "yes") == label:
                row[0] += 1
            else:
                row[2] += 1
                wrong_lines.append(f"{case['id']}: {judgment['id']} read {judgment['answer']} ({judgment['probability']:.2f}), labelled {'yes' if label else 'no'}")
    if sink:
        sink.close()

    print(f"\n{'judgment':20} agree unsettled wrong")
    total = [0, 0, 0]
    for name, row in judged.items():
        print(f"{name:20} {row[0]:5} {row[1]:9} {row[2]:5}")
        total = [a + b for a, b in zip(total, row)]
    print(f"{'all':20} {total[0]:5} {total[1]:9} {total[2]:5}")
    share = land_agree / len(cases) if cases else 0.0
    print(f"\nadvice matched its label in {land_agree} of {len(cases)} cases ({share:.0%}); {tokens} input tokens")
    for line in wrong_lines:
        print(f"wrong: {line}")
    return 0 if total[2] == 0 and share >= args.min_land else 1


if __name__ == "__main__":
    sys.exit(main())
