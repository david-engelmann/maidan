### A suspended workspace gets no email digest

- **Fixed:** the digest sweeper leaves members of a suspended workspace out of its due page and skips them at send time, matching the mail worker, on both backends. Their watermark stays put, so the digest goes out once the suspension is lifted (#1720).
