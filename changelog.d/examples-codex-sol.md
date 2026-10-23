### The Codex recipe runs unattended

- **Fixed:** the Codex claimant recipe names its model (`-m gpt-5.6-sol`; a ChatGPT login refuses models outside its cache), approves the Maidan tools up front so `report_usage` is not refused under `codex exec`, and gives the sandbox network access in its config. The Codex and Cursor CLI prompts take the runbook's one-hour lease instead of fifteen minutes.
