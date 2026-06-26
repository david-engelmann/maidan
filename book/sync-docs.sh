#!/usr/bin/env bash
# Stage the SUMMARY-referenced docs/ pages into book/src/docs/ so mdBook builds
# them as real, in-site pages.
#
# Why this exists: mdBook only builds chapter sources that live under its `src`
# dir. The canonical Maidan docs live in `docs/` (GitHub-native Markdown, so
# they render on github.com without a build step). Before this script, the book
# SUMMARY linked them with `../docs/...` paths that escape `src/`; mdBook
# silently skipped them, so every `docs/*` page in the published sidebar 404'd
# (the links even resolved outside the `/maidan/` base). This copies the curated
# set into `book/src/docs/` and rewrites the few links that point out of `docs/`
# so they resolve on the published site.
#
# The output dir `book/src/docs/` is generated and git-ignored. This runs in
# `.github/workflows/docs.yml` before `mdbook build book`, and must be run before
# any local `mdbook build book` too.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
src_docs="$repo_root/book/src/docs"

rm -rf "$src_docs"
mkdir -p "$src_docs/Gates" "$src_docs/Clusters" "$src_docs/Retros"

# Curated set — exactly the pages referenced by book/src/SUMMARY.md. The rest of
# docs/ (most Clusters/, Retros/, Tracks/, Gates/) is maintainer history and
# deliberately stays out of the published book.
top_pages=(
  "Integration" "Capability Map" "Agent Integration"
  "Production" "Embeddings" "Deploy" "Pi" "Threat-Model" "Glossary"
  "Architecture" "Capabilities" "Decisions" "Conventions" "Operations"
  "Dependencies" "Open Work" "Roadmap" "README"
)
for name in "${top_pages[@]}"; do
  cp "$repo_root/docs/$name.md" "$src_docs/$name.md"
done
cp "$repo_root/docs/Gates/maidan-scale-1.0.md" "$src_docs/Gates/maidan-scale-1.0.md"
cp "$repo_root/docs/Clusters/Cluster A.md"      "$src_docs/Clusters/Cluster A.md"
cp "$repo_root/docs/Retros/README.md"           "$src_docs/Retros/README.md"

# Rewrite links that point OUT of docs/ to repo-root files (CHANGELOG, CLAUDE,
# contracts, rust-toolchain). Those have no page in the book, so point them at
# GitHub so they resolve on the published site instead of 404ing.
gh="https://github.com/david-engelmann/maidan/blob/main"
find "$src_docs" -name '*.md' -print0 | while IFS= read -r -d '' f; do
  perl -pi -e "s{\\]\\(\\.\\./(CHANGELOG\\.md|CLAUDE\\.md|AGENTS\\.md|rust-toolchain\\.toml)\\)}{](${gh}/\$1)}g" "$f"
  perl -pi -e "s{\\]\\(\\.\\./contracts/}{](${gh}/contracts/}g" "$f"
  # Obsidian [[wikilinks]] don't render in mdBook; flatten to readable plain text
  # ([[target|label]] -> label, [[target]] -> target) so maintainer/historical
  # pages don't show broken markup.
  perl -pi -e 's{\[\[([^\]|]+)\|([^\]]+)\]\]}{$2}g' "$f"
  perl -pi -e 's{\[\[([^\]]+)\]\]}{$1}g' "$f"
done

echo "staged $(find "$src_docs" -name '*.md' | wc -l | tr -d ' ') docs pages into book/src/docs/"
