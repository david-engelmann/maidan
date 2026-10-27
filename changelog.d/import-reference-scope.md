### Imports refuse bundles that reach outside their own workspace

- **Fixed:** A workspace restore trusted every id in the bundle, so a signed bundle could attach its threads, messages, pins and memberships to another tenant's members, channels or threads. Every reference must now resolve to an entity the bundle defines in its own workspace; otherwise the import is refused with a validation error and nothing is written. `mode=new` answers such a bundle with 400 instead of panicking.
