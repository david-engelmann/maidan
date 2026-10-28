### maidan-mcp's tests build on dev again

- **Fixed:** a test added by #1718 built its SQLite pool with `SqlitePoolOptions`, which #1726 had just routed through `maidan_search::sqlite_pool_options`, removing the import. Each PR passed on its own; merged together, `maidan-mcp`'s unit tests stopped compiling. The test now uses the shared helper.
