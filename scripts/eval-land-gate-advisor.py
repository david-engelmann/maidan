#!/usr/bin/env python3
"""Measure the advisory land-gate scorer against a labelled JSONL dataset.

Each non-empty line is an object with ``state`` and ``actual`` (green, amber,
or red), plus optional ``instructions`` and ``thresholds``. The script calls
only the advisory endpoint; it never records a land-gate pointer.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

COLORS = ("green", "amber", "red")
INPUT_USD_PER_TOKEN = 0.042 / 1_000_000


def load_cases(path: Path) -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            if not line.strip():
                continue
            try:
                case = json.loads(line)
            except json.JSONDecodeError as error:
                raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
            if not isinstance(case, dict) or "state" not in case:
                raise ValueError(f"{path}:{line_number}: object must contain state")
            if case.get("actual") not in COLORS:
                raise ValueError(
                    f"{path}:{line_number}: actual must be one of {', '.join(COLORS)}"
                )
            cases.append(case)
    if not cases:
        raise ValueError(f"{path}: no evaluation cases")
    return cases


def call_advisor(url: str, thread_id: str, token: str, case: dict[str, Any]) -> dict[str, Any]:
    body = {"state": case["state"]}
    for optional in ("instructions", "thresholds"):
        if optional in case:
            body[optional] = case[optional]
    request = urllib.request.Request(
        f"{url.rstrip('/')}/threads/{thread_id}/land-gate/advice",
        data=json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        detail = error.read().decode(errors="replace")
        raise RuntimeError(f"advisor returned HTTP {error.code}: {detail}") from error


def percentile(values: list[int], quantile: float) -> int:
    ordered = sorted(values)
    index = max(0, math.ceil(quantile * len(ordered)) - 1)
    return ordered[index]


def summarize(rows: list[tuple[str, dict[str, Any]]], bins: int) -> dict[str, Any]:
    correct = 0
    brier = 0.0
    input_tokens = 0
    output_tokens = 0
    latencies: list[int] = []
    buckets: list[list[tuple[float, int]]] = [[] for _ in range(bins)]
    confusion = {actual: {predicted: 0 for predicted in COLORS} for actual in COLORS}

    for actual, advice in rows:
        predicted = advice["raw_land"]
        confidence = float(advice["confidence"])
        probabilities = advice["probabilities"]
        if predicted not in COLORS or not 0 <= confidence <= 1:
            raise ValueError("advisor response has an invalid raw_land or confidence")
        hit = int(predicted == actual)
        correct += hit
        confusion[actual][predicted] += 1
        bucket = min(int(confidence * bins), bins - 1)
        buckets[bucket].append((confidence, hit))
        brier += sum(
            (float(probabilities[color]) - int(color == actual)) ** 2 for color in COLORS
        )
        input_tokens += int(advice["usage"]["input_tokens"])
        output_tokens += int(advice["usage"]["output_tokens"])
        latencies.append(int(advice["latency_ms"]))

    count = len(rows)
    ece = 0.0
    reliability = []
    for index, bucket in enumerate(buckets):
        if not bucket:
            continue
        mean_confidence = statistics.fmean(value[0] for value in bucket)
        accuracy = statistics.fmean(value[1] for value in bucket)
        ece += len(bucket) / count * abs(accuracy - mean_confidence)
        reliability.append(
            {
                "from": index / bins,
                "to": (index + 1) / bins,
                "count": len(bucket),
                "mean_confidence": mean_confidence,
                "accuracy": accuracy,
            }
        )

    return {
        "cases": count,
        "top_choice_accuracy": correct / count,
        "multiclass_brier_score": brier / count,
        "expected_calibration_error": ece,
        "reliability": reliability,
        "confusion": confusion,
        "latency_ms": {
            "p50": percentile(latencies, 0.50),
            "p95": percentile(latencies, 0.95),
            "max": max(latencies),
        },
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "estimated_input_usd_at_0_042_per_million": input_tokens
            * INPUT_USD_PER_TOKEN,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8080")
    parser.add_argument("--thread-id", required=True)
    parser.add_argument("--dataset", required=True, type=Path)
    parser.add_argument("--token", default=os.environ.get("MAIDAN_TOKEN"))
    parser.add_argument("--bins", type=int, default=10)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not args.token:
        parser.error("set MAIDAN_TOKEN or pass --token")
    if args.bins < 2:
        parser.error("--bins must be at least 2")

    try:
        cases = load_cases(args.dataset)
        rows = [
            (case["actual"], call_advisor(args.url, args.thread_id, args.token, case))
            for case in cases
        ]
        result = summarize(rows, args.bins)
    except (OSError, ValueError, RuntimeError) as error:
        print(error, file=sys.stderr)
        return 1

    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
