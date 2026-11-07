### A WASI guest cannot spend the host's memory from a syscall

- **Fixed:** `fd_write` and `random_get` check the guest pointer against the linear memory it was given before the host allocates anything, and they spend fuel for the bytes they scan. A multi-gigabyte length from a 64 KiB guest is an error, not an out-of-memory crash.
- **Fixed:** Function tables are capped the same way linear memory is. A huge initial table, or `table.grow` past the cap, is a memory-limit failure.
- **Fixed:** Scatter/gather counts, signed pointer overflow, a zero-length leading read buffer, unknown clock ids, and `fd_close` of an unknown descriptor follow preview-1: they return an errno and do not panic or report a false end of file.
