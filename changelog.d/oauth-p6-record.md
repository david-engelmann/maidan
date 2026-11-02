### OAuth P6 validation record

- **Added:** `scripts/oauth-p6-record.py` runs discovery, consent, token and `tools/list` against a Maidan instance and writes a record naming the commit, with tokens kept only as SHA-256 prefixes. Its checks are tested against fixtures in CI.
