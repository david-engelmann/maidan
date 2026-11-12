#!/usr/bin/env python3
"""The four SDKs expose every method in the SDK method map (sdk/methods.json).

One map, four hand-written clients: this check fails when one of them omits a
mapped method, when docs/Client Contract.md does not document one, or when a
method with an `example` sends anything but the request the map names.

- Python: imported from sdk/python/src and called offline (a recording
  transport stands in for the network), so a method that exists but sends the
  wrong verb, path or body fails here.
- TypeScript: `node --test sdk/typescript/parity.test.mjs`, the same calls.
- Go and Rust: every mapped name must be defined (`func (s *XService) Name(`,
  `pub fn name(` in the `impl` behind `client.x()`). Their own suites call
  each example (sdk/go/parity_test.go, sdk/rust/tests/parity.rs); those need a
  compiler, so they run with the SDK suites, not here.

Standard library only. `--no-node` skips the TypeScript run (for a machine
without node; CI has it).
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SDK = ROOT / "sdk"
LANGS = ("python", "typescript", "go", "rust")


def load_map() -> list:
    methods = json.loads((SDK / "methods.json").read_text())["methods"]
    seen = set()
    for m in methods:
        if m["op"] in seen:
            raise SystemExit(f"sdk/methods.json: {m['op']} is listed twice")
        seen.add(m["op"])
        for lang in LANGS:
            if not m.get(lang):
                raise SystemExit(f"sdk/methods.json: {m['op']} has no {lang} name")
    return methods


def check_doc(methods: list) -> list:
    doc = (ROOT / "docs" / "Client Contract.md").read_text()
    return [
        f"docs/Client Contract.md does not document `{m['op']}` ({m['http']})"
        for m in methods
        if f"`{m['op']}`" not in doc
    ]


# --- Python -------------------------------------------------------------------


class _Resp:
    def __init__(self, status: int, body):
        self.status = status
        self._raw = b"" if body is None else json.dumps(body).encode()
        self.headers = {}

    def read(self):
        return self._raw

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def check_python(methods: list) -> list:
    sys.path.insert(0, str(SDK / "python" / "src"))
    import maidan  # noqa: E402  (the SDK under test)

    errors = []
    for m in methods:
        client = maidan.Client("http://sdk.invalid", "t", max_retries=0)
        target = client
        for part in m["python"].split("."):
            target = getattr(target, part, None)
        if not callable(target):
            errors.append(f"python: {m['op']} is missing (expected client.{m['python']})")
            continue
        ex = m.get("example")
        if not ex:
            continue
        seen = []

        def urlopen(req, timeout=None, _seen=seen, _reply=ex["reply"]):
            data = req.data
            _seen.append(
                {
                    "method": req.get_method(),
                    "path": req.full_url[len("http://sdk.invalid"):],
                    "body": json.loads(data) if data else None,
                }
            )
            return _Resp(_reply["status"], _reply["body"])

        client._urlopen = urlopen
        try:
            target(*ex["args"], **ex.get("options", {}))
        except Exception as e:  # noqa: BLE001  (any failure is a parity failure)
            errors.append(f"python: {m['op']} raised {type(e).__name__}: {e}")
            continue
        if seen != [ex["request"]]:
            errors.append(f"python: {m['op']} sent {seen}, the map says {ex['request']}")
    return errors


# --- Go -----------------------------------------------------------------------


def check_go(methods: list) -> list:
    src = "\n".join(p.read_text() for p in sorted((SDK / "go").glob("*.go")) if not p.name.endswith("_test.go"))
    defined = set(re.findall(r"^func \(\w+ \*(\w+)\) (\w+)\(", src, re.M))
    fields = set(re.findall(r"^\t(\w+)\s+\*(\w+)Service$", src, re.M))
    errors = []
    for m in methods:
        name = m["go"]
        if "." in name:
            ns, meth = name.split(".")
            if (ns, ns) not in fields:
                errors.append(f"go: {m['op']}: Client has no field {ns} *{ns}Service")
            receiver = ns + "Service"
        else:
            receiver, meth = "Client", name
        if (receiver, meth) not in defined:
            errors.append(f"go: {m['op']} is missing (expected func (*{receiver}) {meth})")
    return errors


# --- Rust ---------------------------------------------------------------------


def _rust_impls(src: str) -> dict:
    """`impl` target -> the names of the `pub fn`s in its blocks."""
    out: dict = {}
    for match in re.finditer(r"^impl(?:<[^>]*>)? (\w+)(?:<[^>]*>)? \{\n(.*?)^\}", src, re.M | re.S):
        out.setdefault(match.group(1), set()).update(re.findall(r"^    pub fn (\w+)", match.group(2), re.M))
    return out


def check_rust(methods: list) -> list:
    src = "\n".join(p.read_text() for p in sorted((SDK / "rust" / "src").glob("*.rs")))
    impls = _rust_impls(src)
    handles = dict(re.findall(r"^    pub fn (\w+)\(&self\) -> (\w+)<'_>", src, re.M))
    errors = []
    for m in methods:
        name = m["rust"]
        if "()." in name:
            handle, meth = name.split("().")
            owner = handles.get(handle)
            if owner is None:
                errors.append(f"rust: {m['op']}: Client has no handle {handle}()")
                continue
        else:
            owner, meth = "Client", name
        if meth not in impls.get(owner, set()):
            errors.append(f"rust: {m['op']} is missing (expected pub fn {meth} on {owner})")
    return errors


def main() -> int:
    methods = load_map()
    errors = check_doc(methods) + check_python(methods) + check_go(methods) + check_rust(methods)
    if "--no-node" not in sys.argv[1:]:
        run = subprocess.run(
            ["node", "--test", "sdk/typescript/parity.test.mjs"], cwd=ROOT, capture_output=True, text=True
        )
        if run.returncode != 0:
            errors.append("typescript: sdk/typescript/parity.test.mjs failed:\n" + run.stdout + run.stderr)
    for e in errors:
        print(e, file=sys.stderr)
    if errors:
        return 1
    examples = sum(1 for m in methods if "example" in m)
    print(f"sdk parity: {len(methods)} methods in all four SDKs; {examples} examples match in Python and TypeScript")
    return 0


if __name__ == "__main__":
    sys.exit(main())
