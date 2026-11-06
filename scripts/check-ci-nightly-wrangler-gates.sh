#!/usr/bin/env bash
# Contract gate for CI/nightly checkout credential pins, darwin release tag
# shape, and Cloudflare container wake cron ≤1×/day.
#
# Usage: scripts/check-ci-nightly-wrangler-gates.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PY'
import re
import sys
from pathlib import Path

errors = []
checked = 0

def require(cond, msg):
    if not cond:
        errors.append(msg)

def checkout_blocks(text: str):
    """Yield each actions/checkout step block, including `name:` before `uses:`."""
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        m = re.match(r'^(\s*)-\s+(?:name:\s*.*|uses:\s*actions/checkout@v\d+\s*)$', line)
        if not m:
            i += 1
            continue
        indent = len(m.group(1))
        # Walk back if this is `name:` then find uses in the same step
        start = i
        block = [line]
        j = i + 1
        saw_checkout = bool(re.search(r'uses:\s*actions/checkout@v\d+', line))
        while j < len(lines):
            nxt = lines[j]
            if nxt.strip() == '':
                break
            nxt_indent = len(nxt) - len(nxt.lstrip(' '))
            if nxt_indent <= indent and re.match(r'^\s*-\s+', nxt):
                break
            if nxt_indent < indent:
                break
            block.append(nxt)
            if re.search(r'uses:\s*actions/checkout@v\d+', nxt):
                saw_checkout = True
            j += 1
        if saw_checkout:
            yield start, '\n'.join(block)
            i = j
            continue
        i += 1

def assert_all_persist_false(rel: str) -> None:
    global checked
    text = Path(rel).read_text()
    checked += 1
    blocks = list(checkout_blocks(text))
    require(len(blocks) >= 1, f'{rel} must use actions/checkout@vN')
    for idx, block in blocks:
        require(
            'persist-credentials: false' in block,
            f'{rel} checkout near line {idx+1} must set persist-credentials: false',
        )

assert_all_persist_false('.github/workflows/ci.yml')
assert_all_persist_false('.github/workflows/nightly.yml')
assert_all_persist_false('.github/workflows/release-darwin-x86.yml')

darwin = Path('.github/workflows/release-darwin-x86.yml').read_text()
checked += 1
require('release tag shape' in darwin,
        'release-darwin-x86.yml must define a release tag shape guard')
require('not a release tag' in darwin,
        'release-darwin-x86.yml must refuse a non-vN.N.N tag')
require(r'v[0-9]+\.[0-9]+\.[0-9]+' in darwin,
        'release-darwin-x86.yml must validate vN.N.N tag shape')
require('contents: read' in darwin,
        'release-darwin-x86.yml must keep contents: read')
require('set -euo pipefail' in darwin,
        'release-darwin-x86.yml run steps must use pipefail')

cron_re = re.compile(r'"crons"\s*:\s*\[\s*"([^"]+)"\s*\]')
for rel in (
    'deploy/cloudflare/wrangler.jsonc',
    'deploy/cloudflare/wrangler.prod.jsonc',
):
    text = Path(rel).read_text()
    checked += 1
    m = cron_re.search(text)
    require(m is not None, f'{rel} must declare a triggers.crons schedule')
    if not m:
        continue
    expr = m.group(1)
    parts = expr.split()
    require(len(parts) == 5, f'{rel} cron must be a 5-field expression, got {expr!r}')
    minute, hour, dom, month, dow = parts

    def single_field(field: str) -> bool:
        # One concrete value only: reject *, steps (*/N), lists (1,2), ranges (0-5).
        return bool(re.fullmatch(r'[0-9]+', field))

    require(single_field(minute),
            f'{rel} cron minute must be one number (not list/range/step), got {expr!r}')
    require(single_field(hour),
            f'{rel} cron hour must be one number (not list/range/step), got {expr!r}')
    require(dom == '*' and month == '*',
            f'{rel} cron should run daily (dom/month *), got {expr}')
    require(dow == '*',
            f'{rel} cron must not restrict weekday (keep ≤1×/day shape), got {expr}')
    require(expr != '0 * * * *', f'{rel} must not use the hourly wake cron')


sdk = Path('scripts/sdk-test.sh').read_text()
checked += 1
require('unsupported language' in sdk,
        'sdk-test.sh must refuse an unsupported language')
require('refusing out-of-range port' in sdk,
        'sdk-test.sh must refuse an out-of-range port')
require('typescript|python|go|rust' in sdk,
        'sdk-test.sh must list the supported languages')

for rel in ('.github/workflows/ci.yml', '.github/workflows/nightly.yml'):
    text = Path(rel).read_text()
    checked += 1
    # Top-level permissions may list keys in any order.
    m = re.search(r'(?m)^permissions:\n((?:  [a-z-]+:[^\n]*\n)+)', text)
    require(m is not None and re.search(r'(?m)^  contents:\s*read\s*$', m.group(1)),
            f'{rel} must keep top-level permissions contents: read')

if errors:
    print('ci-nightly-wrangler gates:', file=sys.stderr)
    for e in errors:
        print(f'  {e}', file=sys.stderr)
    sys.exit(1)
print(f'ci-nightly-wrangler gates: {checked} check(s) ok')
PY
