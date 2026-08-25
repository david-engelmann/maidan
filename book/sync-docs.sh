#!/usr/bin/env bash
# Stage the SUMMARY-referenced docs/ pages into book/src/docs/ so mdBook builds
# them as real, in-site pages — and rewrite their cross-tree links so the
# published site (and the mdbook-linkcheck gate) resolve cleanly.
#
# Why this exists: mdBook only builds chapter sources under its `src` dir. The
# canonical Maidan docs live in `docs/` (GitHub-native Markdown, rendered on
# github.com without a build step). This copies the curated SUMMARY set into
# `book/src/docs/` and fixes up links that would otherwise 404 on the site:
#   * space-named files are staged under hyphenated names (Capability Map.md ->
#     Capability-Map.md) so the published URLs and the link-checker avoid
#     `%20`-in-path friction — links to them are rewritten to match;
#   * links out of the published set (repo-root files, unpublished docs/ pages,
#     repo source) are rewritten to absolute GitHub URLs;
#   * Obsidian [[wikilinks]] are flattened to plain text (mdBook can't render).
#
# The output dir `book/src/docs/` is generated and git-ignored. Runs in
# `.github/workflows/docs.yml` before `mdbook build book`, and must run before
# any local `mdbook build book` too.
#
# NOTE for future docs edits: a new published page must be (1) added to
# `book/src/SUMMARY.md` (hyphenated path if its filename has spaces) and (2)
# listed in the copy set below. A staged page that links to a docs/ page NOT in
# the copy set must have that link GitHub-rewritten here, or the link-checker
# fails.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
src_docs="$repo_root/book/src/docs"

rm -rf "$src_docs"
mkdir -p "$src_docs/Gates" "$src_docs/Clusters" "$src_docs/Retros"

# Curated set — exactly the pages referenced by book/src/SUMMARY.md. The rest of
# docs/ (most Clusters/, Retros/, Tracks/, Gates/) is maintainer history and
# deliberately stays out of the published book. Space-named sources are staged
# under hyphenated names (space -> hyphen).
top_pages=(
  "Integration" "Capability Map" "Agent Integration"
  "Production" "Embeddings" "Deploy" "Pi" "Threat-Model" "Glossary"
  "Architecture" "Capabilities" "Decisions" "Conventions" "Operations"
  "Dependencies" "Open Work" "Roadmap" "README"
  "Providers" "Protocols" "Handoff" "Launch"
  "Pre-Public Hardening" "Path to Impressive" "Expansion Bets"
)
for name in "${top_pages[@]}"; do
  cp "$repo_root/docs/$name.md" "$src_docs/${name// /-}.md"
done
cp "$repo_root/docs/Gates/maidan-scale-1.0.md" "$src_docs/Gates/maidan-scale-1.0.md"
cp "$repo_root/docs/Clusters/Cluster A.md"      "$src_docs/Clusters/Cluster-A.md"
cp "$repo_root/docs/Retros/README.md"           "$src_docs/Retros/README.md"

export GH="https://github.com/david-engelmann/maidan/blob/main"
export GHTREE="https://github.com/david-engelmann/maidan/tree/main"

find "$src_docs" -name '*.md' -print0 | while IFS= read -r -d '' f; do
  # 1) links to repo-root files (no page in the book) -> GitHub
  perl -pi -e 's{\]\(\.\./(CHANGELOG\.md|CLAUDE\.md|AGENTS\.md|rust-toolchain\.toml|deny\.toml)\)}{]($ENV{GH}/$1)}g' "$f"
  perl -pi -e 's{\]\(\.\./contracts/}{]($ENV{GH}/contracts/}g' "$f"
  perl -pi -e 's{\]\(\.\./\.github/}{]($ENV{GH}/.github/}g' "$f"
  perl -pi -e 's{\]\(\.\./\.\./crates/}{]($ENV{GH}/crates/}g' "$f"

  # 2) links to the hyphen-renamed space-files (any `docs/` prefix, %20-encoded)
  perl -pi -e 's{Capability%20Map\.md}{Capability-Map.md}g' "$f"
  perl -pi -e 's{Agent%20Integration\.md}{Agent-Integration.md}g' "$f"
  perl -pi -e 's{Open%20Work\.md}{Open-Work.md}g' "$f"
  perl -pi -e 's{Cluster%20A\.md}{Cluster-A.md}g' "$f"
  perl -pi -e 's{Pre-Public%20Hardening\.md}{Pre-Public-Hardening.md}g' "$f"
  perl -pi -e 's{Path%20to%20Impressive\.md}{Path-to-Impressive.md}g' "$f"
  perl -pi -e 's{Expansion%20Bets\.md}{Expansion-Bets.md}g' "$f"

  # 3) links to docs/ pages that are NOT in the published set -> GitHub
  perl -pi -e 's{\]\((?:\.\./)?(OIDC\.md|Query-Tuning\.md|Post-1\.0\.md)\)}{]($ENV{GH}/docs/$1)}g' "$f"
  perl -pi -e 's{\]\(Presence%20and%20Roster\.md\)}{]($ENV{GH}/docs/Presence%20and%20Roster.md)}g' "$f"
  perl -pi -e 's{\]\(Remaining%20Work\.md\)}{]($ENV{GH}/docs/Remaining%20Work.md)}g' "$f"
  perl -pi -e 's{\]\(Clusters/Product%20Ladder%20102\+\.md\)}{]($ENV{GH}/docs/Clusters/Product%20Ladder%20102+.md)}g' "$f"
  perl -pi -e 's{\]\(Clusters/Product%20Ladder%2077\+\.md\)}{]($ENV{GH}/docs/Clusters/Product%20Ladder%2077+.md)}g' "$f"
  perl -pi -e 's{\]\(Tracks/README\.md\)}{]($ENV{GH}/docs/Tracks/README.md)}g' "$f"
  perl -pi -e 's{\]\(Clusters/\)}{]($ENV{GHTREE}/docs/Clusters/)}g' "$f"
  perl -pi -e 's{\]\(Tracks/\)}{]($ENV{GHTREE}/docs/Tracks/)}g' "$f"

  # 4) Obsidian [[wikilinks]] -> plain text (mdBook can't render them)
  perl -pi -e 's{\[\[([^\]|]+)\|([^\]]+)\]\]}{$2}g' "$f"
  perl -pi -e 's{\[\[([^\]]+)\]\]}{$1}g' "$f"
done

echo "staged $(find "$src_docs" -name '*.md' | wc -l | tr -d ' ') docs pages into book/src/docs/"
