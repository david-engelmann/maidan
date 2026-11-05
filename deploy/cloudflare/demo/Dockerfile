# syntax=docker/dockerfile:1.7
#
# The try.maidan.world demo image. No compile: it layers the release's own
# cosign-verified maidan-server:<tag> image (the stock build, with no bootstrap
# routes) on Debian with bash and openssl, which demo/entrypoint.sh needs to
# make this boot's keys. .github/workflows/deploy-demo.yml passes the image by
# digest. Not for any other deployment.

ARG SERVER_IMAGE

FROM ${SERVER_IMAGE} AS server

# bookworm is Debian 12, the base of the distroless image, so its glibc binary
# runs here unchanged.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends bash ca-certificates openssl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 demo
COPY --from=server /usr/local/bin/maidan-server /usr/local/bin/maidan-server
COPY deploy/cloudflare/demo/entrypoint.sh /usr/local/bin/demo-entrypoint.sh
USER demo
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/demo-entrypoint.sh"]
