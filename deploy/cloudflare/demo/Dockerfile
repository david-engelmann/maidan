# syntax=docker/dockerfile:1.7
#
# The try.maidan.world demo image: maidan-server and the maidan CLI built with
# the bootstrap routes, plus what scripts/demo-board.sh needs (bash, curl, jq).
# demo/entrypoint.sh seeds a fresh SQLite workspace on every boot. Not for any
# other deployment: use crates/maidan-server/Dockerfile.

ARG RUST_VERSION=1.99

FROM node:22-bookworm-slim AS landing
WORKDIR /web
COPY web/landing/package.json web/landing/package-lock.json ./
RUN npm ci
COPY web/landing/ ./
ENV LANDING_OUT=/landing-dist
RUN npm run build

FROM rust:${RUST_VERSION}-slim-bookworm AS builder
WORKDIR /app
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY . .
COPY --from=landing /landing-dist crates/maidan-server/static/landing/dist
ENV MAIDAN_SKIP_LANDING_BUILD=1
RUN cargo build --release --bin maidan-server --bin maidan

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends bash curl jq ca-certificates openssl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 demo
COPY --from=builder /app/target/release/maidan-server /usr/local/bin/maidan-server
COPY --from=builder /app/target/release/maidan /usr/local/bin/maidan
COPY scripts/demo-board.sh /usr/local/bin/demo-board.sh
COPY deploy/cloudflare/demo/entrypoint.sh /usr/local/bin/demo-entrypoint.sh
USER demo
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/demo-entrypoint.sh"]
