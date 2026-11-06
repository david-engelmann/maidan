### Fixed

- **Security:** `oauth-p6-record.py` now fail-closes on userinfo origins, non-https client ids, unsafe `--out` paths, bad commit SHAs, and strips any raw tokens from the written record.
