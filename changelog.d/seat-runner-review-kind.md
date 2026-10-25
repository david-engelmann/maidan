### The seat runner runs review threads through the red-team prompt

- **Added:** `harness.prompts` maps a task kind to its own prompt, so one seat takes changes and reviews alike: a claimed thread whose `attributes.kind` is `review` runs `examples/seat-runner/review-prompt.md`, which follows the red-team runbook and posts the verdict on the pull request. A missing prompt file is refused at startup.
