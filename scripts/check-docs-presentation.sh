#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
html="$root/book/build/html"

if rg -n '\[\[' "$root/docs/Glossary.md" "$root/docs/Production.md" "$root/docs/Threat-Model.md"; then
  echo "published reference pages must use real Markdown links, not wikilinks" >&2
  exit 1
fi

file "$root/docs/assets/maidan-social.png" | rg -q 'PNG image data, 1200 x 630'
test "$(shasum -a 256 "$root/docs/assets/maidan-mark.svg" | cut -d ' ' -f 1)" = \
  "50f55af250bca3351dfb7c4c6fd1fea9bb94899371edb69c315335c7c57b371f"
rg -q 'width="1200" height="630"' "$root/docs/assets/maidan-social.svg"
rg -q 'fill="#14532D"' "$root/docs/assets/maidan-social.svg"
rg -q 'color="#4ADE80"' "$root/docs/assets/maidan-social.svg"
test -f "$html/docs/assets/maidan-mark.svg"
test -f "$html/docs/assets/maidan-social.png"
test -f "$html/favicon.svg"
rg -q 'class="mermaid"' "$html/docs/Architecture.html"
rg -q 'property="og:image".*maidan-social\.png' "$html/index.html"
rg -q 'name="twitter:card".*summary_large_image' "$html/index.html"
if rg -q 'git-edit-button|github\.com/david-engelmann/maidan/edit/main' "$html" --glob '*.html'; then
  echo "published pages still contain the broken generated-source edit link" >&2
  exit 1
fi

echo "docs presentation contract OK"
