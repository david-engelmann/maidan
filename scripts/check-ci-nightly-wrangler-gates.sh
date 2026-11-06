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
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if re.match(r'^\s*- uses: actions/checkout@v4\s*$', line):
            block = [line]
            j = i + 1
            if j < len(lines) and re.match(r'^\s+with:\s*$', lines[j]):
                block.append(lines[j])
                j += 1
                uses_indent = len(line) - len(line.lstrip(' '))
                while j < len(lines):
                    nxt = lines[j]
                    if nxt.strip() == '':
                        break
                    nxt_indent = len(nxt) - len(nxt.lstrip(' '))
                    if nxt_indent > uses_indent and not re.match(r'^\s*-\s', nxt):
                        block.append(nxt)
                        j += 1
                        continue
                    break
            yield i, '\n'.join(block)

def assert_all_persist_false(rel: str) -> None:
    global checked
    text = Path(rel).read_text()
    checked += 1
    blocks = list(checkout_blocks(text))
    require(len(blocks) >= 1, f'{rel} must use actions/checkout@v4')
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
    require(minute != '*' and '/' not in minute,
            f'{rel} cron must not wake more than once per hour ({expr})')
    require(hour != '*' and '/' not in hour and ',' not in hour,
            f'{rel} cron must not wake more than once per day ({expr})')
    require(dom == '*' and month == '*',
            f'{rel} cron should run daily (dom/month *), got {expr}')
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
    require(re.search(r'(?m)^permissions:\n  contents: read\s*$', text),
            f'{rel} must keep top-level permissions contents: read')

if errors:
    print('ci-nightly-wrangler gates:', file=sys.stderr)
    for e in errors:
        print(f'  {e}', file=sys.stderr)
    sys.exit(1)
print(f'ci-nightly-wrangler gates: {checked} check(s) ok')
PY
