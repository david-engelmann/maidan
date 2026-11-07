### A release rerun resumes after its record

- **Fixed:** `cut release` found the release record already on dev only by luck: `grep -m 1` closing the pipe killed `git show` with SIGPIPE, which `pipefail` turned into "no record", so a rerun tried to write the record again and stopped. It now reads the heading without a broken pipe.
