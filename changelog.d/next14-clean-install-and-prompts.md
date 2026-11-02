### Next 14: a clean-machine install job and runnable example prompts

- **Added:** A `clean-machine install` workflow runs `scripts/clean-machine-install.sh` on a fresh runner against each published release (and weekly, and by hand), with no checkout or source build, failing when the quickstart pins a different release.
- **Added:** `listings/prompts.json` holds the example prompts per listing with the tools each should pick, and `scripts/listing-prompts.py` runs them through a platform's own model and prints the validation record's table. CI checks every expected tool is in the MCP catalog.
