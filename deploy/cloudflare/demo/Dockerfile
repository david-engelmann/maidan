# syntax=docker/dockerfile:1.7
#
# The try.maidan.world demo image. No compile: it layers the release's own
# cosign-verified images, maidan-server:<tag>-demo (the only published build
# with the bootstrap routes) and maidan-cli:<tag>, on Debian with what
# scripts/demo-board.sh needs (bash, curl, jq). demo/entrypoint.sh seeds a
# fresh SQLite workspace on every boot. .github/workflows/deploy-demo.yml
# passes both images by digest. Not for any other deployment.

ARG SERVER_IMAGE
ARG CLI_IMAGE

FROM ${SERVER_IMAGE} AS server
FROM ${CLI_IMAGE} AS cli

# bookworm is Debian 12, the base of both distroless images, so their glibc
# binaries run here unchanged.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends bash curl jq ca-certificates openssl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 demo
COPY --from=server /usr/local/bin/maidan-server /usr/local/bin/maidan-server
COPY --from=cli /usr/local/bin/maidan /usr/local/bin/maidan
COPY scripts/demo-board.sh /usr/local/bin/demo-board.sh
COPY deploy/cloudflare/demo/entrypoint.sh /usr/local/bin/demo-entrypoint.sh
USER demo
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/demo-entrypoint.sh"]
