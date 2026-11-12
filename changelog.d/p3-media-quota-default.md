### The default media quota is held by tests

- **Fixed:** a test on both store backends and one over REST now fail if
  `MAIDAN_MEDIA_QUOTA_BYTES` unset stops meaning the documented 20 GiB
  per-workspace cap. A workspace with no quota of its own is refused a media
  upload past the default, another workspace is not, and an explicit `0` or
  `off` still lifts it (OPS-004). The default itself has shipped since #1366.
