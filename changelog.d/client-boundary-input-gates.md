### Fixed

- **Fixed:** Client and demo boundary inputs fail closed: TypeScript SDK cache/usage helpers reject blank or non-string ids/providers and ignore inherited usage fields, the console CSP checker flags `data:` script/style URLs plus inline `style=` and `srcdoc`, and the try demo normalizes `MAIDAN_PUBLIC_ORIGIN` to a credential-free `https` origin (falling back to `https://try.maidan.world` when mis-set).
