# syntax=docker/dockerfile:1

FROM --platform=linux/amd64 rust:1-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates

RUN cargo build --release -p local-deploy-server

FROM --platform=linux/amd64 debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends nginx ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /data /var/log/nginx /run/nginx

COPY --from=builder /build/target/release/local-deploy-server /usr/local/bin/local-deploy-server

EXPOSE 11000 11001
VOLUME ["/data"]

ENV LOCAL_DEPLOY_PUBLIC_ADDRESS=http://localhost:11000

ENTRYPOINT ["/usr/local/bin/local-deploy-server"]
