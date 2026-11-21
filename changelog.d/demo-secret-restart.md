### Docs: when the demo picks up a new provider key

- **Fixed:** `docs/Deploy.md` said the next request after a provider key is set on the demo Worker recycles its container. A key set while the demo is running reaches it when the container next starts, after fifteen idle minutes; until then the try page offers the provider and an ask of it gets no answer.
