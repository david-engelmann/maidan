### Fixed

- **Fixed:** The try.maidan.world Worker rejects non-http redirects, disallowed methods, and Host headers that do not match `MAIDAN_PUBLIC_ORIGIN`; the demo entrypoint refuses unsafe artifact roots; `deploy-release` only health-checks `https://*.maidan.world`; the console gains runtime API shape gates; and the Python SDK fails closed on oversize strings/lists and deep nesting in `Model.from_dict`.
