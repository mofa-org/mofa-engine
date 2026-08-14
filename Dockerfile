# Multi-stage Dockerfile for MoFA Engine Gateway
FROM rust:1.80-slim-bookworm AS builder

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY mofa-kernel ./mofa-kernel
COPY mofa-engine-core ./mofa-engine-core
COPY mofa-engine-sdk ./mofa-engine-sdk
COPY mofa-engine-app ./mofa-engine-app
COPY mofa-observability ./mofa-observability

RUN cargo build --release --bin mofa-engine

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/mofa-engine /usr/local/bin/mofa-engine

EXPOSE 8420
ENV MOFA_PORT=8420
ENV RUST_LOG=info

CMD ["/usr/local/bin/mofa-engine"]
