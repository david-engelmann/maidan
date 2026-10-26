### Embedding repair honours the allowance cutoff

- **Fixed:** the embedding repair sweep no longer embeds messages in an open sign-up workspace that is past its `embedding_calls` allowance, and every message it does embed now counts toward that workspace's `embedding_calls`, the same as the live indexer. Before, the sweep embedded unvectored messages in every workspace on its five-minute timer without checking the cutoff or metering the work.
