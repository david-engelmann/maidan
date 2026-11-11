#!/usr/bin/env bash
# Install ripgrep for CI without depending on the Ubuntu archive: on
# 2026-11-11 the runners could not resolve ubuntu.com for hours and every
# job that ran apt-get failed. The release tarball comes from GitHub, which
# the runner reaches anyway, and is checked against a pinned SHA-256.
set -euo pipefail

if command -v rg >/dev/null 2>&1; then
  rg --version | head -n 1
  exit 0
fi

version=14.1.1
name="ripgrep-${version}-x86_64-unknown-linux-musl"
sha256=4cf9f2741e6c465ffdb7c26f38056a59e2a2544b51f7cc128ef28337eeae4d8e
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT

curl -fsSL --retry 5 --retry-all-errors -o "$dir/rg.tgz" \
  "https://github.com/BurntSushi/ripgrep/releases/download/${version}/${name}.tar.gz"
echo "${sha256}  $dir/rg.tgz" | sha256sum -c -
tar -xzf "$dir/rg.tgz" -C "$dir"
sudo install -m 0755 "$dir/${name}/rg" /usr/local/bin/rg
rg --version | head -n 1
