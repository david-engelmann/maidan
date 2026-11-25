# The image Glama builds to run Maidan in its sandbox and list its tools
# (glama.ai/mcp/servers/david-engelmann/maidan). It installs the newest signed
# release binary, checked against that release's SHA256SUMS, and serves MCP
# over stdio on a fresh local SQLite database with a workspace made at start.
# No key, token or network service is needed, and nothing leaves the container.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$(dpkg --print-architecture)" in \
      amd64) triple=x86_64-unknown-linux-gnu ;; \
      arm64) triple=aarch64-unknown-linux-gnu ;; \
      *) echo "no Maidan release for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    base=https://github.com/david-engelmann/maidan/releases/latest/download; \
    cd /tmp; \
    curl -fsSLO "$base/maidan-$triple.tar.gz"; \
    curl -fsSLO "$base/SHA256SUMS"; \
    grep " maidan-$triple.tar.gz\$" SHA256SUMS | sha256sum -c -; \
    mkdir x; tar -xzf "maidan-$triple.tar.gz" -C x; \
    install -m 0755 "$(find x -name maidan -type f | head -1)" /usr/local/bin/maidan; \
    rm -rf x maidan-*.tar.gz SHA256SUMS; \
    printf '%s\n' \
      '#!/bin/sh' \
      'set -eu' \
      'export DATABASE_URL="${DATABASE_URL:-sqlite:///tmp/maidan.db?mode=rwc}"' \
      'export MAIDAN_ALLOW_INSECURE_DEV_KEK=1' \
      'MAIDAN_MCP_TOKEN=$(maidan init --workspace sandbox --admin-handle admin 2>/dev/null | sed -nE "s/^ +(maid_[A-Za-z0-9_-]+)\$/\1/p" | head -1)' \
      'export MAIDAN_MCP_TOKEN' \
      'exec maidan mcp-stdio' \
      > /usr/local/bin/maidan-mcp; \
    chmod 0755 /usr/local/bin/maidan-mcp; \
    maidan --version
USER nobody
CMD ["/usr/local/bin/maidan-mcp"]
