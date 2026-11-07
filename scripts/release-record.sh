#!/usr/bin/env bash
# The release record for X.Y.Z, written into the working tree in one pass:
#   - CHANGELOG.md: [Unreleased] and every changelog.d fragment folded into
#     `## [X.Y.Z] — today`, with a summary paragraph;
#   - docs/Capabilities.md: a `## [vX.Y.Z]` record, newest first;
#   - CLAUDE.md: the `latest **`vX.Y.Z`**` line;
#   - every deploy pin (the files scripts/check-deploy-pins.sh lists) moved to
#     vX.Y.Z, and the stack chart repackaged.
# The pins move here, before the tag exists, because nothing they name is
# hashed into the tree: docker/Dockerfile.quickstart checks its tarball against
# the SHA256SUMS that release publishes. So the record commit is the release
# commit, and no pins PR follows the release.
#
# Usage: scripts/release-record.sh X.Y.Z [summary sentence]
# The summary opens the CHANGELOG section and the Capabilities record; without
# one, both are written from the fragments' headings. The cut-release workflow
# runs this; it works the same in a local checkout, and never commits.
set -euo pipefail
cd "$(dirname "$0")/.."

die() {
  echo "release record: $*" >&2
  exit 1
}

version="${1:-}"
summary="${2:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: $0 X.Y.Z [summary]"
tag="v${version}"
git rev-parse -q --verify "refs/tags/${tag}" >/dev/null && die "${tag} is already tagged"
grep -q "^## \[${version//./\\.}\]" CHANGELOG.md && die "CHANGELOG.md already has [${version}]"
command -v helm >/dev/null || die "helm is needed to repackage helm/maidan-stack/charts"

# The release the pins name now; every one of them moves from it.
from="$(sed -nE 's/^  tag: (v[0-9]+\.[0-9]+\.[0-9]+)$/\1/p' helm/maidan/values-prod.yaml)"
[[ -n "$from" ]] || die "could not read the current pin from helm/maidan/values-prod.yaml"
[[ "$from" != "$tag" ]] || die "the pins already name ${tag}"

# The headings this release carries, in the order the CHANGELOG will list them.
headings_file="$(mktemp)"
trap 'rm -f "$headings_file"' EXIT
{
  awk '/^## \[Unreleased\]/{on=1; next} on && /^## \[/{exit} on && /^### /' CHANGELOG.md
  bash scripts/changelog-fragments.sh preview | grep '^### ' || true
} | grep -vxE '### (Added|Changed|Fixed|Removed|Security|Deprecated)' \
  | sed 's/^### //' >"$headings_file" || true
[[ -s "$headings_file" ]] || die "nothing to release: [Unreleased] and changelog.d/ are empty"

bash scripts/changelog-fragments.sh release "$version" >/dev/null

today="$(date -u +%Y-%m-%d)"
python3 - "$version" "$today" "$from" "$headings_file" "$summary" <<'PY'
import re
import sys

version, today, old, headings_path, summary = sys.argv[1:6]
tag = f"v{version}"
headings = [h.strip() for h in open(headings_path) if h.strip()]


def plain(text):
    # A bare [x] in a published doc reads as a broken link to mdbook-linkcheck.
    return text.replace("[", "(").replace("]", ")")


def lower_first(text):
    # "The README", "A release": lowered. "API keys", "MCP": kept.
    if text[:1].isupper() and (len(text) == 1 or text[1].islower() or text[1] == " "):
        return text[0].lower() + text[1:]
    return text


summary = " ".join(summary.split())
if summary and not summary.endswith((".", "!", "?")):
    summary += "."
if not summary:
    names = [lower_first(plain(h)).rstrip(".") for h in headings]
    if len(names) == 1:
        summary = f"This release carries one change: {names[0]}."
    else:
        summary = f"This release carries {len(names)} changes: " + "; ".join(names) + "."


def edit(path, fn):
    text = open(path).read()
    new = fn(text)
    if new == text:
        sys.exit(f"release record: {path} did not change")
    open(path, "w").write(new)


# CHANGELOG: the summary paragraph under the new section's heading.
head = f"## [{version}] — {today}\n"


def changelog(text):
    if text.count(head) != 1:
        sys.exit(f"release record: CHANGELOG.md has no single '{head.strip()}'")
    return text.replace(head, head + "\n" + summary + "\n", 1)


edit("CHANGELOG.md", changelog)

# Capabilities: the record goes above the newest record of either form.
title = lower_first(plain(headings[0]))
if len(headings) > 1:
    title += f", and {len(headings) - 1} more"
bullets = "\n".join(f"- {plain(h)}" for h in headings)
record = (
    f"## [{tag}](https://github.com/david-engelmann/maidan/releases/tag/{tag}) — {title}\n\n"
    f"{summary}\n\n{bullets}\n\n"
    f"`CHANGELOG.md` under `[{version}]` has every entry.\n\n"
)


def capabilities(text):
    m = re.search(r"^## (\[v[0-9]|Cluster [0-9])", text, re.M)
    if not m:
        sys.exit("release record: docs/Capabilities.md has no record to go above")
    return text[: m.start()] + record + text[m.start():]


edit("docs/Capabilities.md", capabilities)

# CLAUDE.md: the one line scripts/check-release-records.sh reads.
latest = re.compile(r"latest \*\*`v[0-9]+\.[0-9]+\.[0-9]+`\*\* \([0-9]{4}-[0-9]{2}-[0-9]{2}\)")


def guide(text):
    if len(latest.findall(text)) != 1:
        sys.exit("release record: CLAUDE.md needs one 'latest **`vX.Y.Z`** (YYYY-MM-DD)'")
    return latest.sub(f"latest **`{tag}`** ({today})", text, count=1)


edit("CLAUDE.md", guide)

# Every deploy pin (scripts/check-deploy-pins.sh checks the result).
pins = [
    "compose.quickstart.yaml",
    "docker/Dockerfile.quickstart",
    "docs/Pi.md",
    "docs/Production.md",
    "docs/Quickstart.md",
    "helm/maidan-stack/values-prod.yaml",
    "helm/maidan/values-prod.yaml",
    "k8s/overlays/prod/kustomization.yaml",
    "listings/docker/server.yaml",
    "listings/registry/server.json",
    "listings/toolhive/server.json",
]
old_re = re.compile(re.escape(old) + r"(?![0-9])")
for path in pins:
    def move(text):
        text = old_re.sub(tag, text)
        if path == "listings/registry/server.json":
            text = text.replace(f'"version": "{old[1:]}"', f'"version": "{version}"')
        return text

    edit(path, move)
PY

helm package helm/maidan -d helm/maidan-stack/charts >/dev/null

bash scripts/changelog-fragments.sh check >/dev/null
bash scripts/check-release-records.sh
bash scripts/check-deploy-pins.sh >/dev/null
echo "release record: ${tag} written; the pins moved from ${from}"
